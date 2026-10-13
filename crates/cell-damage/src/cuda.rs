//! CUDA row damage through the driver API, loaded at run time.
//!
//! `libcuda.so.1` comes with the NVIDIA driver and `libnvrtc` with the CUDA
//! toolkit. Both are opened with `dlopen`, and the kernel is compiled from
//! source by NVRTC for the detected device, so building needs neither.
//!
//! Grids stay on the device by generation. A comparison uploads the baseline
//! only when it is not already resident, then only the candidate rows of the
//! next grid, and reads back eight bytes per candidate row.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::time::Instant;

use super::residency::{GridKey, OverBudget, Residency};
use super::{checked_height, CudaError, DeviceTimings, GridRef, PackedCell, RowDamage, ShapeError};

type CuResult = c_int;
type CuDevice = c_int;
type CuContext = *mut c_void;
type CuModule = *mut c_void;
type CuFunction = *mut c_void;
type CuDevicePtr = u64;
type NvrtcResult = c_int;
type NvrtcProgram = *mut c_void;

const KERNEL_SOURCE: &str = include_str!("row_damage.cu");
const KERNEL_NAME: &CStr = c"row_damage";
const BLOCK_THREADS: c_uint = 128;
const ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR: c_int = 75;
const ATTRIBUTE_COMPUTE_CAPABILITY_MINOR: c_int = 76;
const MAX_RESIDENT_GRIDS: usize = 16;
const MAX_RESIDENT_BYTES: usize = 128 << 20;
const CELL_BYTES: usize = std::mem::size_of::<PackedCell>();
const DRIVER_LIBRARIES: &[&CStr] = &[c"libcuda.so.1", c"libcuda.so"];
const NVRTC_LIBRARIES: &[&CStr] = &[c"libnvrtc.so.13", c"libnvrtc.so.12", c"libnvrtc.so"];

struct Driver {
    init: unsafe extern "C" fn(c_uint) -> CuResult,
    device_get: unsafe extern "C" fn(*mut CuDevice, c_int) -> CuResult,
    device_get_name: unsafe extern "C" fn(*mut c_char, c_int, CuDevice) -> CuResult,
    device_get_attribute: unsafe extern "C" fn(*mut c_int, c_int, CuDevice) -> CuResult,
    primary_ctx_retain: unsafe extern "C" fn(*mut CuContext, CuDevice) -> CuResult,
    primary_ctx_release: unsafe extern "C" fn(CuDevice) -> CuResult,
    ctx_set_current: unsafe extern "C" fn(CuContext) -> CuResult,
    ctx_synchronize: unsafe extern "C" fn() -> CuResult,
    module_load_data: unsafe extern "C" fn(*mut CuModule, *const c_void) -> CuResult,
    module_unload: unsafe extern "C" fn(CuModule) -> CuResult,
    module_get_function: unsafe extern "C" fn(*mut CuFunction, CuModule, *const c_char) -> CuResult,
    mem_alloc: unsafe extern "C" fn(*mut CuDevicePtr, usize) -> CuResult,
    mem_free: unsafe extern "C" fn(CuDevicePtr) -> CuResult,
    memcpy_htod: unsafe extern "C" fn(CuDevicePtr, *const c_void, usize) -> CuResult,
    memcpy_dtoh: unsafe extern "C" fn(*mut c_void, CuDevicePtr, usize) -> CuResult,
    memcpy_dtod: unsafe extern "C" fn(CuDevicePtr, CuDevicePtr, usize) -> CuResult,
    #[allow(clippy::type_complexity)] // Mirrors cuLaunchKernel's C signature.
    launch_kernel: unsafe extern "C" fn(
        CuFunction,
        c_uint,
        c_uint,
        c_uint,
        c_uint,
        c_uint,
        c_uint,
        c_uint,
        *mut c_void,
        *mut *mut c_void,
        *mut *mut c_void,
    ) -> CuResult,
    get_error_name: unsafe extern "C" fn(CuResult, *mut *const c_char) -> CuResult,
}

struct Nvrtc {
    create_program: unsafe extern "C" fn(
        *mut NvrtcProgram,
        *const c_char,
        *const c_char,
        c_int,
        *const *const c_char,
        *const *const c_char,
    ) -> NvrtcResult,
    compile_program: unsafe extern "C" fn(NvrtcProgram, c_int, *const *const c_char) -> NvrtcResult,
    get_ptx_size: unsafe extern "C" fn(NvrtcProgram, *mut usize) -> NvrtcResult,
    get_ptx: unsafe extern "C" fn(NvrtcProgram, *mut c_char) -> NvrtcResult,
    get_program_log_size: unsafe extern "C" fn(NvrtcProgram, *mut usize) -> NvrtcResult,
    get_program_log: unsafe extern "C" fn(NvrtcProgram, *mut c_char) -> NvrtcResult,
    destroy_program: unsafe extern "C" fn(*mut NvrtcProgram) -> NvrtcResult,
}

fn open_library(names: &[&CStr]) -> Result<*mut c_void, CudaError> {
    let mut failures = Vec::new();
    for name in names {
        // SAFETY: `name` is NUL-terminated; the handle is kept for the process lifetime.
        let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if !handle.is_null() {
            return Ok(handle);
        }
        failures.push(dl_error());
    }
    Err(CudaError {
        step: "load library",
        detail: failures.join("; "),
    })
}

fn dl_error() -> String {
    // SAFETY: dlerror returns null or a NUL-terminated thread-local message.
    let message = unsafe { libc::dlerror() };
    if message.is_null() {
        "unknown dlopen error".into()
    } else {
        // SAFETY: checked non-null above.
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    }
}

/// # Safety
/// `F` must be the `extern "C"` function pointer type of the named symbol.
unsafe fn symbol<F: Copy>(library: *mut c_void, name: &CStr) -> Result<F, CudaError> {
    debug_assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<*mut c_void>());
    let address = libc::dlsym(library, name.as_ptr());
    if address.is_null() {
        return Err(CudaError {
            step: "resolve symbol",
            detail: name.to_string_lossy().into_owned(),
        });
    }
    Ok(std::mem::transmute_copy::<*mut c_void, F>(&address))
}

impl Driver {
    fn load() -> Result<Self, CudaError> {
        let library = open_library(DRIVER_LIBRARIES)?;
        // SAFETY: each type matches the CUDA driver API declaration in cuda.h.
        unsafe {
            Ok(Self {
                init: symbol(library, c"cuInit")?,
                device_get: symbol(library, c"cuDeviceGet")?,
                device_get_name: symbol(library, c"cuDeviceGetName")?,
                device_get_attribute: symbol(library, c"cuDeviceGetAttribute")?,
                primary_ctx_retain: symbol(library, c"cuDevicePrimaryCtxRetain")?,
                primary_ctx_release: symbol(library, c"cuDevicePrimaryCtxRelease_v2")?,
                ctx_set_current: symbol(library, c"cuCtxSetCurrent")?,
                ctx_synchronize: symbol(library, c"cuCtxSynchronize")?,
                module_load_data: symbol(library, c"cuModuleLoadData")?,
                module_unload: symbol(library, c"cuModuleUnload")?,
                module_get_function: symbol(library, c"cuModuleGetFunction")?,
                mem_alloc: symbol(library, c"cuMemAlloc_v2")?,
                mem_free: symbol(library, c"cuMemFree_v2")?,
                memcpy_htod: symbol(library, c"cuMemcpyHtoD_v2")?,
                memcpy_dtoh: symbol(library, c"cuMemcpyDtoH_v2")?,
                memcpy_dtod: symbol(library, c"cuMemcpyDtoD_v2")?,
                launch_kernel: symbol(library, c"cuLaunchKernel")?,
                get_error_name: symbol(library, c"cuGetErrorName")?,
            })
        }
    }

    fn check(&self, step: &'static str, result: CuResult) -> Result<(), CudaError> {
        if result == 0 {
            return Ok(());
        }
        let mut name = std::ptr::null();
        // SAFETY: cuGetErrorName stores a static string or leaves the pointer null.
        unsafe { (self.get_error_name)(result, &mut name) };
        let detail = if name.is_null() {
            format!("CUresult {result}")
        } else {
            // SAFETY: checked non-null; the driver owns the static string.
            let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
            format!("{name} ({result})")
        };
        Err(CudaError { step, detail })
    }
}

impl Nvrtc {
    fn load() -> Result<Self, CudaError> {
        let library = open_library(NVRTC_LIBRARIES)?;
        // SAFETY: each type matches the NVRTC declaration in nvrtc.h.
        unsafe {
            Ok(Self {
                create_program: symbol(library, c"nvrtcCreateProgram")?,
                compile_program: symbol(library, c"nvrtcCompileProgram")?,
                get_ptx_size: symbol(library, c"nvrtcGetPTXSize")?,
                get_ptx: symbol(library, c"nvrtcGetPTX")?,
                get_program_log_size: symbol(library, c"nvrtcGetProgramLogSize")?,
                get_program_log: symbol(library, c"nvrtcGetProgramLog")?,
                destroy_program: symbol(library, c"nvrtcDestroyProgram")?,
            })
        }
    }

    /// Compiles the kernel to NUL-terminated PTX for one compute capability.
    fn compile(&self, (major, minor): (c_int, c_int)) -> Result<Vec<u8>, CudaError> {
        let failed = |step, result: NvrtcResult| CudaError {
            step,
            detail: format!("nvrtcResult {result}"),
        };
        let source = CString::new(KERNEL_SOURCE).map_err(|_| CudaError {
            step: "kernel source",
            detail: "interior NUL".into(),
        })?;
        let architecture = CString::new(format!("--gpu-architecture=compute_{major}{minor}"))
            .map_err(|_| CudaError {
                step: "kernel options",
                detail: "interior NUL".into(),
            })?;
        let options = [architecture.as_ptr()];
        let mut program = std::ptr::null_mut();
        // SAFETY: all pointers are valid NUL-terminated strings for the call.
        let result = unsafe {
            (self.create_program)(
                &mut program,
                source.as_ptr(),
                c"row_damage.cu".as_ptr(),
                0,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if result != 0 {
            return Err(failed("nvrtcCreateProgram", result));
        }
        let compiled = (|| {
            // SAFETY: `program` was created above; `options` outlives the call.
            let result = unsafe { (self.compile_program)(program, 1, options.as_ptr()) };
            if result != 0 {
                let mut error = failed("nvrtcCompileProgram", result);
                error.detail = format!("{}: {}", error.detail, self.log(program));
                return Err(error);
            }
            let mut size = 0;
            // SAFETY: `program` compiled successfully.
            let result = unsafe { (self.get_ptx_size)(program, &mut size) };
            if result != 0 {
                return Err(failed("nvrtcGetPTXSize", result));
            }
            let mut ptx = vec![0u8; size];
            // SAFETY: `ptx` has the size NVRTC reported, including the NUL.
            let result = unsafe { (self.get_ptx)(program, ptx.as_mut_ptr().cast()) };
            if result != 0 {
                return Err(failed("nvrtcGetPTX", result));
            }
            Ok(ptx)
        })();
        // SAFETY: `program` is destroyed exactly once.
        unsafe { (self.destroy_program)(&mut program) };
        compiled
    }

    fn log(&self, program: NvrtcProgram) -> String {
        let mut size = 0;
        // SAFETY: `program` is a live NVRTC program.
        if unsafe { (self.get_program_log_size)(program, &mut size) } != 0 || size == 0 {
            return String::new();
        }
        let mut log = vec![0u8; size];
        // SAFETY: `log` has the size NVRTC reported.
        if unsafe { (self.get_program_log)(program, log.as_mut_ptr().cast()) } != 0 {
            return String::new();
        }
        String::from_utf8_lossy(&log)
            .trim_end_matches('\0')
            .trim()
            .to_owned()
    }
}

struct DeviceBuffer {
    ptr: CuDevicePtr,
    bytes: usize,
}

/// A CUDA context with the compiled row damage kernel and resident grids.
pub struct CudaDamage {
    driver: Driver,
    device: CuDevice,
    context: CuContext,
    module: CuModule,
    function: CuFunction,
    device_name: String,
    compute_capability: (i32, i32),
    residency: Residency<CuDevicePtr>,
    staging: Option<DeviceBuffer>,
    row_list: Option<DeviceBuffer>,
    damage: Option<DeviceBuffer>,
    host_rows: Vec<u32>,
    host_staged: Vec<PackedCell>,
    host_damage: Vec<RowDamage>,
}

// SAFETY: the context handle is process-global (the device's primary context)
// and every method makes it current on the calling thread before any driver
// call; `&mut self` serializes all use of the handles.
unsafe impl Send for CudaDamage {}

impl CudaDamage {
    /// Loads the driver and NVRTC, selects device 0 and compiles the kernel.
    pub fn open() -> Result<Self, CudaError> {
        let driver = Driver::load()?;
        // SAFETY: plain driver API calls with valid out-pointers.
        unsafe {
            driver.check("cuInit", (driver.init)(0))?;
            let mut device = 0;
            driver.check("cuDeviceGet", (driver.device_get)(&mut device, 0))?;
            let mut name = [0 as c_char; 256];
            driver.check(
                "cuDeviceGetName",
                (driver.device_get_name)(name.as_mut_ptr(), name.len() as c_int, device),
            )?;
            let device_name = CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned();
            let mut major = 0;
            let mut minor = 0;
            driver.check(
                "cuDeviceGetAttribute",
                (driver.device_get_attribute)(
                    &mut major,
                    ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR,
                    device,
                ),
            )?;
            driver.check(
                "cuDeviceGetAttribute",
                (driver.device_get_attribute)(
                    &mut minor,
                    ATTRIBUTE_COMPUTE_CAPABILITY_MINOR,
                    device,
                ),
            )?;
            let ptx = Nvrtc::load()?.compile((major, minor))?;
            let mut context = std::ptr::null_mut();
            driver.check(
                "cuDevicePrimaryCtxRetain",
                (driver.primary_ctx_retain)(&mut context, device),
            )?;
            let mut opened = Self {
                driver,
                device,
                context,
                module: std::ptr::null_mut(),
                function: std::ptr::null_mut(),
                device_name,
                compute_capability: (major, minor),
                residency: Residency::new(MAX_RESIDENT_GRIDS, MAX_RESIDENT_BYTES),
                staging: None,
                row_list: None,
                damage: None,
                host_rows: Vec::new(),
                host_staged: Vec::new(),
                host_damage: Vec::new(),
            };
            opened.make_current()?;
            opened.driver.check(
                "cuModuleLoadData",
                (opened.driver.module_load_data)(&mut opened.module, ptx.as_ptr().cast()),
            )?;
            opened.driver.check(
                "cuModuleGetFunction",
                (opened.driver.module_get_function)(
                    &mut opened.function,
                    opened.module,
                    KERNEL_NAME.as_ptr(),
                ),
            )?;
            Ok(opened)
        }
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn compute_capability(&self) -> (i32, i32) {
        self.compute_capability
    }

    /// Number of grids currently kept on the device.
    pub fn resident_grids(&self) -> usize {
        self.residency.len()
    }

    /// The GPU counterpart of [`super::cpu_row_damage`], with the same result.
    ///
    /// When `rows` is given, the next grid kept on the device is the baseline
    /// with those rows replaced, which is exact under the same guarantee the
    /// CPU reference relies on: unlisted rows equal the baseline.
    pub fn row_damage(
        &mut self,
        prev: GridRef<'_>,
        next: GridRef<'_>,
        rows: Option<&[u32]>,
        out: &mut Vec<RowDamage>,
    ) -> Result<DeviceTimings, CudaError> {
        let result = self.compare(prev, next, rows, out);
        if result.is_err() {
            // A failed step can leave a resident grid half written.
            for ptr in self.residency.drain() {
                self.free(ptr);
            }
        }
        result
    }

    fn compare(
        &mut self,
        prev: GridRef<'_>,
        next: GridRef<'_>,
        rows: Option<&[u32]>,
        out: &mut Vec<RowDamage>,
    ) -> Result<DeviceTimings, CudaError> {
        let height = checked_height(prev, next)?;
        if rows.is_some_and(|rows| rows.iter().any(|&row| row as usize >= height)) {
            return Err(ShapeError::RowOutOfRange.into());
        }
        out.clear();
        out.resize(height, RowDamage::UNCHANGED);
        self.host_rows.clear();
        match rows {
            Some(rows) => self.host_rows.extend_from_slice(rows),
            None => self.host_rows.extend(0..height as u32),
        }
        let mut timings = DeviceTimings::default();
        if self.host_rows.is_empty() || next.width == 0 {
            return Ok(timings);
        }
        let width = next.width;
        let row_count = self.host_rows.len();
        let grid_bytes = next.cells.len() * CELL_BYTES;
        self.make_current()?;
        self.residency.tick();
        let key = |grid: GridRef<'_>| GridKey {
            generation: grid.generation,
            width: grid.width,
            cells: grid.cells.len(),
        };
        let (prev_key, next_key) = (key(prev), key(next));

        let upload_started = Instant::now();
        let prev_ptr = match self.resident_ptr(prev_key) {
            Some(ptr) => ptr,
            None => {
                let ptr = self.insert_resident(prev_key, grid_bytes, None)?;
                self.upload(ptr, prev.cells, &mut timings)?;
                ptr
            }
        };
        let (staged_ptr, next_out_ptr, compact) = match self.resident_ptr(next_key) {
            Some(next_ptr) => (next_ptr, 0, 0u32),
            None if row_count == height => {
                let next_ptr = self.insert_resident(next_key, grid_bytes, Some(prev_key))?;
                self.upload(next_ptr, next.cells, &mut timings)?;
                (next_ptr, 0, 0)
            }
            None => {
                let next_ptr = self.insert_resident(next_key, grid_bytes, Some(prev_key))?;
                // SAFETY: both device buffers hold `grid_bytes`.
                let result = unsafe { (self.driver.memcpy_dtod)(next_ptr, prev_ptr, grid_bytes) };
                self.driver.check("cuMemcpyDtoD", result)?;
                self.host_staged.clear();
                for &row in &self.host_rows {
                    let start = row as usize * width;
                    self.host_staged
                        .extend_from_slice(&next.cells[start..start + width]);
                }
                let staging = Self::ensure_buffer(
                    &self.driver,
                    &mut self.staging,
                    row_count * width * CELL_BYTES,
                )?;
                let staged = std::mem::take(&mut self.host_staged);
                let uploaded = self.upload(staging, &staged, &mut timings);
                self.host_staged = staged;
                uploaded?;
                (staging, next_ptr, 1)
            }
        };
        let row_list = Self::ensure_buffer(&self.driver, &mut self.row_list, row_count * 4)?;
        let rows_host = std::mem::take(&mut self.host_rows);
        let uploaded = self.upload(row_list, &rows_host, &mut timings);
        self.host_rows = rows_host;
        uploaded?;
        let damage_ptr = Self::ensure_buffer(
            &self.driver,
            &mut self.damage,
            row_count * std::mem::size_of::<RowDamage>(),
        )?;
        timings.upload = upload_started.elapsed();

        let launch_started = Instant::now();
        let mut prev_arg = prev_ptr;
        let mut staged_arg = staged_ptr;
        let mut next_out_arg = next_out_ptr;
        let mut rows_arg = row_list;
        let mut row_count_arg = row_count as c_uint;
        let mut width_arg = width as c_uint;
        let mut compact_arg = compact;
        let mut damage_arg = damage_ptr;
        let mut params: [*mut c_void; 8] = [
            (&mut prev_arg as *mut CuDevicePtr).cast(),
            (&mut staged_arg as *mut CuDevicePtr).cast(),
            (&mut next_out_arg as *mut CuDevicePtr).cast(),
            (&mut rows_arg as *mut CuDevicePtr).cast(),
            (&mut row_count_arg as *mut c_uint).cast(),
            (&mut width_arg as *mut c_uint).cast(),
            (&mut compact_arg as *mut c_uint).cast(),
            (&mut damage_arg as *mut CuDevicePtr).cast(),
        ];
        // SAFETY: `params` matches the kernel signature in row_damage.cu and
        // every device pointer covers the rows the kernel reads or writes.
        unsafe {
            let result = (self.driver.launch_kernel)(
                self.function,
                row_count as c_uint,
                1,
                1,
                BLOCK_THREADS,
                1,
                1,
                0,
                std::ptr::null_mut(),
                params.as_mut_ptr(),
                std::ptr::null_mut(),
            );
            self.driver.check("cuLaunchKernel", result)?;
            self.driver
                .check("cuCtxSynchronize", (self.driver.ctx_synchronize)())?;
        }
        timings.launch_sync = launch_started.elapsed();

        let readback_started = Instant::now();
        self.host_damage.clear();
        self.host_damage.resize(row_count, RowDamage::UNCHANGED);
        // SAFETY: `host_damage` holds `row_count` RowDamage values, the size the
        // kernel wrote; RowDamage is two repr(C) u32 values like the kernel's pairs.
        let result = unsafe {
            (self.driver.memcpy_dtoh)(
                self.host_damage.as_mut_ptr().cast(),
                damage_ptr,
                row_count * std::mem::size_of::<RowDamage>(),
            )
        };
        self.driver.check("cuMemcpyDtoH", result)?;
        for (&row, &damage) in self.host_rows.iter().zip(&self.host_damage) {
            out[row as usize] = damage;
        }
        timings.readback = readback_started.elapsed();
        Ok(timings)
    }

    fn make_current(&self) -> Result<(), CudaError> {
        // SAFETY: `context` is the retained primary context.
        let result = unsafe { (self.driver.ctx_set_current)(self.context) };
        self.driver.check("cuCtxSetCurrent", result)
    }

    fn upload<T: Copy>(
        &self,
        ptr: CuDevicePtr,
        values: &[T],
        timings: &mut DeviceTimings,
    ) -> Result<(), CudaError> {
        let bytes = std::mem::size_of_val(values);
        if bytes == 0 {
            return Ok(());
        }
        // SAFETY: the caller sized the device buffer for `values`.
        let result = unsafe { (self.driver.memcpy_htod)(ptr, values.as_ptr().cast(), bytes) };
        self.driver.check("cuMemcpyHtoD", result)?;
        timings.uploaded_bytes += bytes as u64;
        Ok(())
    }

    fn resident_ptr(&mut self, key: GridKey) -> Option<CuDevicePtr> {
        let mut stale = Vec::new();
        let ptr = self.residency.get(key, &mut stale);
        for ptr in stale {
            self.free(ptr);
        }
        ptr
    }

    /// Allocates a resident grid within the residency bounds, evicting least
    /// recently used grids other than `keep`. A grid that cannot fit is an
    /// error, so the caller falls back to the CPU instead of exceeding them.
    fn insert_resident(
        &mut self,
        key: GridKey,
        bytes: usize,
        keep: Option<GridKey>,
    ) -> Result<CuDevicePtr, CudaError> {
        let mut evicted = Vec::new();
        let reserved = self.residency.reserve(bytes, keep, &mut evicted);
        for ptr in evicted {
            self.free(ptr);
        }
        reserved.map_err(|OverBudget| CudaError {
            step: "device residency",
            detail: format!("a {bytes}-byte grid exceeds the {MAX_RESIDENT_BYTES}-byte bound"),
        })?;
        let ptr = self.alloc(bytes)?;
        self.residency.insert(key, ptr, bytes);
        Ok(ptr)
    }

    fn ensure_buffer(
        driver: &Driver,
        slot: &mut Option<DeviceBuffer>,
        bytes: usize,
    ) -> Result<CuDevicePtr, CudaError> {
        if let Some(buffer) = slot.as_ref().filter(|buffer| buffer.bytes >= bytes) {
            return Ok(buffer.ptr);
        }
        if let Some(buffer) = slot.take() {
            // SAFETY: allocated by cuMemAlloc and no longer referenced.
            unsafe { (driver.mem_free)(buffer.ptr) };
        }
        let bytes = bytes.max(1).next_power_of_two();
        let mut ptr = 0;
        // SAFETY: valid out-pointer.
        driver.check("cuMemAlloc", unsafe { (driver.mem_alloc)(&mut ptr, bytes) })?;
        *slot = Some(DeviceBuffer { ptr, bytes });
        Ok(ptr)
    }

    fn alloc(&self, bytes: usize) -> Result<CuDevicePtr, CudaError> {
        let mut ptr = 0;
        // SAFETY: valid out-pointer.
        let result = unsafe { (self.driver.mem_alloc)(&mut ptr, bytes.max(1)) };
        self.driver.check("cuMemAlloc", result)?;
        Ok(ptr)
    }

    fn free(&self, ptr: CuDevicePtr) {
        // SAFETY: allocated by cuMemAlloc and no longer referenced.
        unsafe { (self.driver.mem_free)(ptr) };
    }
}

impl Drop for CudaDamage {
    fn drop(&mut self) {
        if self.make_current().is_err() {
            return;
        }
        for ptr in self.residency.drain() {
            self.free(ptr);
        }
        for buffer in [
            self.staging.take(),
            self.row_list.take(),
            self.damage.take(),
        ]
        .into_iter()
        .flatten()
        {
            self.free(buffer.ptr);
        }
        // SAFETY: the module and the primary context reference are released once.
        unsafe {
            if !self.module.is_null() {
                (self.driver.module_unload)(self.module);
            }
            (self.driver.primary_ctx_release)(self.device);
        }
    }
}
