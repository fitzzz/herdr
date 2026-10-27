//! External launch authority, independent from native agent/session identity.

use serde::{Deserialize, Serialize};

use crate::api::schema::AgentLauncher;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BoundAgentLauncher {
    pub launcher: AgentLauncher,
    #[serde(rename = "kind")]
    pub agent: String,
    pub session: Option<crate::agent_resume::AgentSessionRef>,
    pub argv: Vec<String>,
    pub argv_origin: LauncherArgvOrigin,
    /// Only a current-process native report can confirm a restored/admitted binding.
    #[serde(skip)]
    pub observed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LauncherArgvOrigin {
    Admission,
    CanonicalResume,
}

#[cfg(test)]
pub(crate) struct TestDirectory(std::path::PathBuf);

#[cfg(test)]
impl TestDirectory {
    pub(crate) fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "herdr-launcher-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.0
    }
}

#[cfg(test)]
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl AgentLauncher {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !std::path::Path::new(&self.executable).is_absolute()
            || self.executable.chars().any(char::is_control)
        {
            return Err(
                "launcher executable must be an absolute path without control characters".into(),
            );
        }
        if self.prefix_args.len() > 256
            || self.env.len() > 256
            || self
                .prefix_args
                .iter()
                .any(|arg| arg.chars().any(char::is_control))
            || self.env.iter().any(|(key, value)| {
                let mut bytes = key.bytes();
                !bytes
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
                    || bytes.any(|c| !c.is_ascii_alphanumeric() && c != b'_')
                    || value.chars().any(char::is_control)
            })
            || self.executable.len()
                + self.prefix_args.iter().map(String::len).sum::<usize>()
                + self
                    .env
                    .iter()
                    .map(|(key, value)| key.len() + value.len())
                    .sum::<usize>()
                > 65536
        {
            return Err("launcher arguments or environment are invalid or too large".into());
        }
        if !crate::platform::launcher_executable_available(std::path::Path::new(&self.executable)) {
            return Err("launcher executable is missing or not executable".into());
        }
        Ok(())
    }

    pub(crate) fn argv(&self, native_args: &[String]) -> Vec<String> {
        std::iter::once(self.executable.clone())
            .chain(self.prefix_args.iter().cloned())
            .chain(native_args.iter().cloned())
            .collect()
    }

    pub(crate) fn command(&self, native_args: &[String], shell: &str) -> Result<String, String> {
        self.validate()?;
        crate::platform::agent_launcher_command(&self.argv(native_args), &self.env, shell)
            .ok_or_else(|| "launcher cannot be encoded for the target shell".into())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn agent_launcher_exec_preserves_arguments_and_parent_environment() {
        let args = vec![
            "",
            "white space",
            "single'quote",
            "double\"quote",
            "$(touch /never)",
            "`literal`",
            "--",
            "same-uuid",
        ];
        let launcher = AgentLauncher {
            executable: "/usr/bin/python3".into(),
            prefix_args: vec![
                "-c".into(),
                "import os,sys,json;print(json.dumps([sys.argv[1:],os.environ['LAUNCHER_TEST']]))"
                    .into(),
            ],
            env: [("LAUNCHER_TEST".into(), "selected ' $()".into())].into(),
        };
        let command = launcher
            .command(
                &args.iter().map(|arg| (*arg).into()).collect::<Vec<_>>(),
                "bash",
            )
            .unwrap();
        let output = std::process::Command::new("bash")
            .args([
                "--noprofile",
                "--norc",
                "-c",
                &format!("{command}\nprintf '%s' \"$LAUNCHER_TEST\""),
            ])
            .env("LAUNCHER_TEST", "parent")
            .output()
            .unwrap();
        assert!(output.status.success());
        let output = String::from_utf8(output.stdout).unwrap();
        let (child, parent) = output.split_once('\n').unwrap();
        let child: serde_json::Value = serde_json::from_str(child).unwrap();
        assert_eq!(child, serde_json::json!([args, "selected ' $()"]));
        assert_eq!(parent, "parent");
    }

    #[test]
    fn agent_launcher_rejects_relative_non_executable_and_invalid_env() {
        let mut launcher = AgentLauncher {
            executable: "authority".into(),
            prefix_args: vec![],
            env: Default::default(),
        };
        assert!(launcher.validate().is_err());
        launcher.executable = "/".into();
        assert!(launcher.validate().is_err());
        launcher.executable = "/usr/bin/printf".into();
        launcher.env.insert("INVALID=KEY".into(), "value".into());
        assert!(launcher.validate().is_err());
    }
}
