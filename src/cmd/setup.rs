//! First-run setup and config maintenance: `init`, `config …`, `doctor`.

use crate::config::{self, Config, UNIT_PATH};
use crate::t3::T3;
use anyhow::{Result, bail};
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Default)]
pub struct InitArgs {
    pub inbox: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub projects: Vec<String>,
}

/// Merge `init` flags into `cfg`. Projects are added (deduplicated), never removed.
pub fn apply_init(cfg: &mut Config, a: InitArgs) -> Result<()> {
    if let Some(i) = a.inbox {
        cfg.inbox_project = Some(config::expand_home(&config::normalize_project(&i)?));
    }
    if a.provider.is_some() || a.model.is_some() {
        cfg.set("provider", a.provider)?;
        cfg.set("model", a.model)?;
    }
    add_projects(cfg, &a.projects)?;
    Ok(())
}

pub fn add_projects(cfg: &mut Config, projects: &[String]) -> Result<Vec<String>> {
    let mut added = Vec::new();
    for p in projects {
        let p = config::expand_home(&config::normalize_project(p)?);
        if !cfg.projects.contains(&p) {
            cfg.projects.push(p.clone());
            added.push(p);
        }
    }
    Ok(added)
}

pub fn rm_projects(cfg: &mut Config, projects: &[String]) -> Result<()> {
    for p in projects {
        // Match the stored form, or the path as normalized now (if it still exists).
        let candidates = [config::expand_home(p), config::normalize_project(p).map(|n| config::expand_home(&n)).unwrap_or_default()];
        let before = cfg.projects.len();
        cfg.projects.retain(|x| !candidates.contains(x));
        if cfg.projects.len() == before {
            bail!("{p} is not in projects");
        }
    }
    Ok(())
}

/// If `t3` is on this shell's PATH but not on the PATH the timers use, pin `t3_bin`.
/// Returns the path it pinned.
pub fn pin_t3_bin(cfg: &mut Config) -> Option<String> {
    if cfg.t3_bin.is_some() || config::find_in("t3", UNIT_PATH.iter().copied()).is_some() {
        return None;
    }
    let found = config::which("t3")?.to_string_lossy().into_owned();
    cfg.t3_bin = Some(found.clone());
    Some(found)
}

/// Ask on stderr for anything `args` leaves open. Only when stdin and stderr are TTYs.
pub fn ask_missing(cfg: &Config, args: &mut InitArgs) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Ok(());
    }
    let stdin = std::io::stdin();
    let ask = |q: &str, current: Option<String>| -> Result<Option<String>> {
        match &current {
            Some(c) => eprint!("{q} [{c}]: "),
            None => eprint!("{q}: "),
        }
        std::io::stderr().flush()?;
        let mut line = String::new();
        stdin.lock().read_line(&mut line)?;
        let line = line.trim();
        Ok(if line.is_empty() { None } else { Some(line.to_string()) })
    };
    let shown = |v: &Option<String>| v.as_deref().map(config::contract_home);
    if args.inbox.is_none() {
        args.inbox = ask("Inbox project for the daily 📋 thread (path inside it, or id)", shown(&cfg.inbox_project))?;
    }
    if args.projects.is_empty() {
        let current = (!cfg.projects.is_empty())
            .then(|| cfg.projects.iter().map(|p| config::contract_home(p)).collect::<Vec<_>>().join(" "));
        let q = if current.is_some() { "More projects to sweep (space-separated paths)" } else { "Projects to sweep (space-separated paths)" };
        if let Some(line) = ask(q, current.map(|c| format!("keep: {c}")))? {
            args.projects = line.split_whitespace().map(str::to_string).collect();
        }
    }
    if args.provider.is_none() && args.model.is_none() {
        if let Some(p) = ask("Inbox provider, e.g. antigravity (empty = project default)", cfg.provider.clone())? {
            args.provider = Some(p);
            args.model = ask("Inbox model, e.g. gemini-3.8-flash", cfg.model.clone())?;
        }
    }
    Ok(())
}

// ---- doctor ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug)]
pub struct Check {
    pub level: Level,
    pub what: String,
    pub detail: String,
}

fn check(level: Level, what: impl Into<String>, detail: impl Into<String>) -> Check {
    Check { level, what: what.into(), detail: detail.into() }
}

/// Checks that need the T3 server: inbox and every project resolve.
pub fn check_t3(cfg: &Config, t3: &dyn T3) -> Vec<Check> {
    let mut out = Vec::new();
    match &cfg.inbox_project {
        None => out.push(check(Level::Fail, "inbox_project", "not set → t3bot init --inbox <path or id>")),
        Some(p) => match t3.list(p) {
            Ok(_) => out.push(check(Level::Ok, "inbox_project", config::contract_home(p))),
            Err(e) => out.push(check(Level::Fail, "inbox_project", format!("{}: {e:#}", config::contract_home(p)))),
        },
    }
    if cfg.inbox_project.as_ref().is_some_and(|i| cfg.projects.contains(i)) {
        out.push(check(Level::Warn, "inbox_project", "is also in projects; the inbox threads will be swept too"));
    }
    if cfg.projects.is_empty() {
        out.push(check(Level::Warn, "projects", "none configured, nothing to sweep → t3bot config add-project <path>"));
    }
    for p in &cfg.projects {
        let p_ = config::contract_home(p);
        match t3.list(p) {
            Ok(v) => out.push(check(Level::Ok, "project", format!("{p_} ({} threads)", v.len()))),
            Err(e) => out.push(check(Level::Fail, "project", format!("{p_}: {e:#}"))),
        }
    }
    match (&cfg.provider, &cfg.model) {
        (Some(p), Some(m)) => out.push(check(Level::Ok, "inbox model", format!("{p} / {m}"))),
        (None, None) => out.push(check(Level::Ok, "inbox model", "project default")),
        _ => out.push(check(Level::Warn, "inbox model", "set both provider and model, or neither (one alone is ignored)")),
    }
    out
}

/// Version string if `bin --version` runs.
fn version_of(bin: &str) -> Option<String> {
    let out = Command::new(bin).arg("--version").output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Local checks: config file, binaries, skill, timers. Returns (checks, t3 usable).
pub fn check_host(cfg: &Config, config_path: &Path) -> (Vec<Check>, bool) {
    let mut out = Vec::new();
    if config_path.exists() {
        out.push(check(Level::Ok, "config", config::contract_home(&config_path.to_string_lossy())));
    } else {
        out.push(check(Level::Fail, "config", format!("{} missing → t3bot init", config_path.display())));
    }

    let bin = cfg.t3_bin();
    let t3_ok = match version_of(&bin) {
        Some(v) => {
            let at = config::which(&bin).map(|p| p.to_string_lossy().into_owned()).unwrap_or(bin.clone());
            out.push(check(Level::Ok, "t3", format!("{v} ({})", config::contract_home(&at))));
            true
        }
        None => {
            out.push(check(Level::Fail, "t3", format!("cannot run `{bin} --version` → install T3 Code, or t3bot config set t3_bin <path>")));
            false
        }
    };
    if t3_ok && cfg.t3_bin.is_none() && config::find_in("t3", UNIT_PATH.iter().copied()).is_none() {
        out.push(check(Level::Fail, "t3 for timers", "not in the timers' PATH → t3bot init (pins t3_bin)"));
    }
    if config::find_in("t3bot", UNIT_PATH.iter().copied()).is_none() {
        out.push(check(Level::Warn, "t3bot for timers", "not in ~/.local/bin, /usr/local/bin, /usr/bin or /bin; the timers will not find it"));
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let skills: Vec<_> = [".claude/skills", ".agents/skills", ".gemini/antigravity-cli/skills"]
        .iter()
        .filter(|d| Path::new(&home).join(d).join("t3bot/SKILL.md").exists())
        .map(|d| format!("~/{d}"))
        .collect();
    if skills.is_empty() {
        out.push(check(Level::Warn, "@t3bot skill", "not installed → rerun install.sh"));
    } else {
        out.push(check(Level::Ok, "@t3bot skill", skills.join(", ")));
    }

    let systemd = Command::new("systemctl").args(["--user", "is-enabled", "t3bot-morning.timer", "t3bot-tick.timer"]).output();
    match systemd {
        Ok(o) if o.status.success() => out.push(check(Level::Ok, "timers", "t3bot-morning, t3bot-tick enabled")),
        Ok(_) => out.push(check(Level::Warn, "timers", "not enabled → rerun install.sh, or schedule `t3bot tick` yourself")),
        Err(_) => out.push(check(Level::Warn, "timers", "no systemd; schedule `t3bot tick` yourself")),
    }
    let user = std::env::var("USER").unwrap_or_default();
    if let Ok(o) = Command::new("loginctl").args(["show-user", &user, "-p", "Linger", "--value"]).output()
        && o.status.success()
        && String::from_utf8_lossy(&o.stdout).trim() == "no"
    {
        out.push(check(Level::Warn, "linger", format!("off: timers stop when you log out → loginctl enable-linger {user}")));
    }
    (out, t3_ok)
}

pub fn render_checks(checks: &[Check]) -> String {
    let mut s = String::new();
    for c in checks {
        let mark = match c.level {
            Level::Ok => "✓",
            Level::Warn => "!",
            Level::Fail => "✗",
        };
        s += &format!("{mark} {:<18} {}\n", c.what, c.detail);
    }
    s
}
