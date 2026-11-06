//! `~/.t3bot/config.toml`. Only a tiny TOML subset is accepted (string values and
//! arrays of strings, `#` comments) so we don't need a TOML dependency.
//!
//! Paths may start with `~/`; they are expanded when the config is loaded, and written
//! back in `~/` form so one file can be copied between machines.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// PATH the systemd units give `t3bot tick` (`contrib/systemd/t3bot-tick@.service`).
/// A `t3` outside these needs `t3_bin`.
pub const UNIT_PATH: &[&str] = &["~/.local/bin", "/usr/local/bin", "/usr/bin", "/bin"];

/// Keys `t3bot config set` accepts (`projects` has its own commands).
pub const KEYS: &[&str] = &["inbox_project", "provider", "model", "t3_bin"];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    pub inbox_project: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub projects: Vec<String>,
    pub t3_bin: Option<String>,
}

/// The example `t3bot init --print` shows.
pub fn example() -> String {
    Config {
        inbox_project: Some("~/inbox".into()),
        provider: Some("antigravity".into()),
        model: Some("gemini-3.8-flash".into()),
        projects: vec!["~/code/project-a".into(), "~/code/project-b".into()],
        t3_bin: None,
    }
    .render()
}

pub fn home() -> PathBuf {
    if let Ok(h) = std::env::var("T3BOT_HOME") {
        return PathBuf::from(h);
    }
    user_home().join(".t3bot")
}

pub fn path() -> PathBuf {
    home().join("config.toml")
}

fn user_home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/root".into()))
}

/// `~/x` → `$HOME/x`. Anything else is returned unchanged.
pub fn expand_home(s: &str) -> String {
    match s.strip_prefix("~/") {
        Some(rest) => user_home().join(rest).to_string_lossy().into_owned(),
        None if s == "~" => user_home().to_string_lossy().into_owned(),
        None => s.to_string(),
    }
}

/// `$HOME/x` → `~/x`, for writing portable configs.
pub fn contract_home(s: &str) -> String {
    let home = user_home();
    match Path::new(s).strip_prefix(&home) {
        Ok(rest) if home != Path::new("/") => {
            if rest.as_os_str().is_empty() { "~".into() } else { format!("~/{}", rest.display()) }
        }
        _ => s.to_string(),
    }
}

/// A project given on the command line: a path (made absolute, must exist, stored as
/// `~/…` when under $HOME) or a T3 project id (kept as is).
pub fn normalize_project(s: &str) -> Result<String> {
    let s = s.trim();
    let looks_like_path = s.contains('/') || s.starts_with('~') || s.starts_with('.');
    if !looks_like_path {
        return Ok(s.to_string());
    }
    let p = PathBuf::from(expand_home(s));
    let abs = std::fs::canonicalize(&p).with_context(|| format!("{} does not exist", p.display()))?;
    anyhow::ensure!(abs.is_dir(), "{} is not a directory", abs.display());
    Ok(contract_home(&abs.to_string_lossy()))
}

/// First executable named `name` in `dirs`.
pub fn find_in<'a>(name: &str, dirs: impl IntoIterator<Item = &'a str>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    dirs.into_iter().filter(|d| !d.is_empty()).map(|d| Path::new(&expand_home(d)).join(name)).find(|p| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").unwrap_or_default();
    find_in(name, path.split(':'))
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let cfg = match std::fs::read_to_string(path) {
            Ok(s) => Config::parse(&s).with_context(|| format!("parsing {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(cfg.expanded())
    }

    /// `~/` expanded in every path-like value.
    pub fn expanded(mut self) -> Config {
        self.inbox_project = self.inbox_project.map(|s| expand_home(&s));
        self.t3_bin = self.t3_bin.map(|s| expand_home(&s));
        self.projects = self.projects.iter().map(|s| expand_home(s)).collect();
        self
    }

    /// The `t3` binary to run: `t3_bin`, else `t3` from PATH.
    pub fn t3_bin(&self) -> String {
        self.t3_bin.clone().unwrap_or_else(|| "t3".into())
    }

    pub fn set(&mut self, key: &str, value: Option<String>) -> Result<()> {
        let value = value.filter(|v| !v.trim().is_empty());
        let slot = match key {
            "inbox_project" => &mut self.inbox_project,
            "provider" => &mut self.provider,
            "model" => &mut self.model,
            "t3_bin" => &mut self.t3_bin,
            "projects" => bail!("use `t3bot config add-project` / `rm-project` for projects"),
            _ => bail!("unknown key {key:?} (keys: {}, projects)", KEYS.join(", ")),
        };
        *slot = value;
        Ok(())
    }

    /// The config file, with comments. Values are written in `~/` form.
    pub fn render(&self) -> String {
        let line = |key: &str, v: &Option<String>, placeholder: &str| match v {
            Some(v) => format!("{key} = {}\n", quote(&contract_home(v))),
            None => format!("# {key} = {}\n", quote(placeholder)),
        };
        let mut s = String::from(
            "# t3bot config. Edit by hand, or with `t3bot config set|add-project|rm-project`;\n\
             # check it with `t3bot doctor`. Values are strings; paths may start with `~/`.\n\n\
             # Project that holds the daily 📋 inbox thread (id or a path inside it).\n\
             # Use a project that is not in `projects` below.\n",
        );
        s += &line("inbox_project", &self.inbox_project, "~/inbox");
        s += "\n# Model for the inbox agent: provider instance id + model slug, as in\n\
              # `t3 thread start --provider/--model`. Unset = the project's default.\n\
              # `t3 thread start` launches in the project's default runtime mode (full-access unless\n\
              # changed in T3 settings), and Claude refuses full-access when T3 runs as root.\n";
        s += &line("provider", &self.provider, "antigravity");
        s += &line("model", &self.model, "gemini-3.8-flash");
        s += "\n# Projects to sweep (paths inside each project, or project ids).\n";
        if self.projects.is_empty() {
            s += "projects = []\n";
        } else {
            s += "projects = [\n";
            for p in &self.projects {
                s += &format!("  {},\n", quote(&contract_home(p)));
            }
            s += "]\n";
        }
        s += "\n# Path to the t3 binary. Only needed if `t3` is not in ~/.local/bin, /usr/local/bin,\n\
              # /usr/bin or /bin, the PATH the systemd timers run with.\n";
        s += &line("t3_bin", &self.t3_bin, "/opt/t3/bin/t3");
        s
    }

    /// Write atomically (temp file + rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, self.render()).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
    }

    pub fn parse(src: &str) -> Result<Config> {
        let mut cfg = Config::default();
        let mut lines = src.lines().enumerate();
        while let Some((n, line)) = lines.next() {
            let line = strip_comment(line).trim().to_string();
            if line.is_empty() {
                continue;
            }
            let Some((key, val)) = line.split_once('=') else {
                bail!("line {}: expected key = value", n + 1);
            };
            let key = key.trim();
            let mut val = val.trim().to_string();
            if val.starts_with('[') {
                while !val.contains(']') {
                    let Some((_, more)) = lines.next() else { bail!("unterminated array for {key}") };
                    val.push_str(strip_comment(more).trim());
                }
                let inner = val.trim_start_matches('[').trim_end_matches(']');
                let items = inner
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(unquote)
                    .collect::<Result<Vec<_>>>()?;
                match key {
                    "projects" => cfg.projects = items,
                    _ => bail!("unknown array key {key}"),
                }
            } else {
                let v = unquote(&val)?;
                match key {
                    "inbox_project" => cfg.inbox_project = Some(v),
                    "provider" => cfg.provider = Some(v),
                    "model" => cfg.model = Some(v),
                    "t3_bin" => cfg.t3_bin = Some(v),
                    _ => bail!("unknown key {key}"),
                }
            }
        }
        Ok(cfg)
    }
}

fn strip_comment(line: &str) -> &str {
    // '#' outside of a quoted string starts a comment.
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(s: &str) -> Result<String> {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        Ok(s[1..s.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\"))
    } else {
        bail!("expected a quoted string, got {s}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_example() {
        let c = Config::parse(&example()).unwrap();
        assert_eq!(c.projects.len(), 2);
        assert_eq!(c.provider.as_deref(), Some("antigravity"));
        assert!(c.t3_bin.is_none());
    }

    #[test]
    fn render_roundtrips() {
        let c = Config {
            inbox_project: Some("00000000-inbox".into()),
            provider: None,
            model: Some("m \"q\" \\x".into()),
            projects: vec![],
            t3_bin: Some("/opt/t3".into()),
        };
        assert_eq!(Config::parse(&c.render()).unwrap(), c);
        assert_eq!(Config::parse(&Config::default().render()).unwrap(), Config::default());
    }

    #[test]
    fn home_paths() {
        let home = user_home().to_string_lossy().into_owned();
        assert_eq!(expand_home("~/a/b"), format!("{home}/a/b"));
        assert_eq!(expand_home("/x/~/y"), "/x/~/y");
        if home != "/" {
            assert_eq!(contract_home(&format!("{home}/a")), "~/a");
        }
        assert_eq!(contract_home("/elsewhere/a"), "/elsewhere/a");
        let c = Config { projects: vec!["~/p".into()], ..Default::default() }.expanded();
        assert_eq!(c.projects, vec![format!("{home}/p")]);
    }

    #[test]
    fn set_validates_keys() {
        let mut c = Config::default();
        c.set("provider", Some("x".into())).unwrap();
        assert_eq!(c.provider.as_deref(), Some("x"));
        c.set("provider", Some(" ".into())).unwrap();
        assert!(c.provider.is_none());
        assert!(c.set("projects", None).is_err());
        assert!(c.set("nope", None).is_err());
    }
}
