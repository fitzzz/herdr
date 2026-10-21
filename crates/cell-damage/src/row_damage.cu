// Row damage between a resident baseline grid and candidate rows of the next grid.
// Must match cell_damage::cpu_row_damage exactly; compiled at run time by NVRTC.

struct Cell {
    unsigned long long symbol;
    unsigned long long colors;
    unsigned long long rest;
};

// One block per candidate row. damage[2 * i] is the first changed column and
// damage[2 * i + 1] one past the last, or 0xffffffff and 0 when the row is equal.
// With compact != 0, staged row i holds grid row rows[i]; otherwise staged is a
// whole grid. A non-null next_out receives the staged rows at their grid rows.
extern "C" __global__ void row_damage(
    const Cell* prev,
    const Cell* staged,
    Cell* next_out,
    const unsigned int* rows,
    unsigned int row_count,
    unsigned int width,
    unsigned int compact,
    unsigned int* damage)
{
    __shared__ unsigned int first;
    __shared__ unsigned int end;
    const unsigned int i = blockIdx.x;
    if (i >= row_count) {
        return;
    }
    if (threadIdx.x == 0) {
        first = 0xffffffffu;
        end = 0u;
    }
    __syncthreads();
    const unsigned long long grid_base = (unsigned long long)rows[i] * width;
    const unsigned long long staged_base =
        compact ? (unsigned long long)i * width : grid_base;
    for (unsigned int x = threadIdx.x; x < width; x += blockDim.x) {
        const Cell p = prev[grid_base + x];
        const Cell n = staged[staged_base + x];
        if (next_out) {
            next_out[grid_base + x] = n;
        }
        if (p.symbol != n.symbol || p.colors != n.colors || p.rest != n.rest) {
            atomicMin(&first, x);
            atomicMax(&end, x + 1u);
        }
    }
    __syncthreads();
    if (threadIdx.x == 0) {
        damage[2u * i] = first;
        damage[2u * i + 1u] = end;
    }
}
