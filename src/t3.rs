//! All T3 access goes through `trait T3`. The real implementation shells out to the
//! installed `t3` CLI (never with `--wait`); tests use `FakeT3`.

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadInfo {
    pub thread_id: String,
    pub project_id: String,
    pub title: String,
    #[serde(default)]
    pub link: Option<String>,
    pub status: String,
    #[serde(default)]
    pub latest_run_id: Option<String>,
    #[serde(default)]
    pub settled: bool,
    #[serde(default)]
    pub snoozed: bool,
    #[serde(default)]
    pub item_count: u64,
}

impl ThreadInfo {
    /// D5: `<latestRunId>:<itemCount>`. settle/pin/rename don't change it.
    pub fn fingerprint(&self) -> String {
        format!("{}:{}", self.latest_run_id.as_deref().unwrap_or(""), self.item_count)
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThreadShow {
    pub thread: serde_json::Value,
    #[serde(default)]
    pub messages: Vec<Message>,
}

pub struct StartArgs<'a> {
    pub project: &'a str,
    pub message: &'a str,
    pub title: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
}

pub trait T3 {
    fn list(&self, project: &str) -> Result<Vec<ThreadInfo>>;
    fn show(&self, thread_id: &str, messages: u32) -> Result<ThreadShow>;
    fn send(&self, thread_id: &str, message: &str) -> Result<()>;
    /// Returns the new thread id.
    fn start(&self, args: &StartArgs) -> Result<String>;
    // organize: not in the 0.1.7 CLI. Settling is done by the inbox agent via MCP
    // `t3_thread_organize` (design §4).
}

pub struct CliT3 {
    pub bin: String,
}

impl CliT3 {
    fn run(&self, args: &[&str], stdin: Option<&str>) -> Result<String> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args)
            .current_dir("/")
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().with_context(|| format!("running {}", self.bin))?;
        if let Some(input) = stdin {
            child.stdin.take().unwrap().write_all(input.as_bytes())?;
        }
        let out = child.wait_with_output()?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let err = if err.trim().is_empty() { String::from_utf8_lossy(&out.stdout) } else { err };
            bail!("t3 {} failed: {}", args.first().copied().unwrap_or("") , err.trim());
        }
        Ok(String::from_utf8(out.stdout)?)
    }
}

impl T3 for CliT3 {
    fn list(&self, project: &str) -> Result<Vec<ThreadInfo>> {
        let out = self.run(&["thread", "list", "--project", project, "--json"], None)?;
        serde_json::from_str(&out).context("parsing t3 thread list --json")
    }

    fn show(&self, thread_id: &str, messages: u32) -> Result<ThreadShow> {
        let n = messages.to_string();
        let out = self.run(&["thread", "show", thread_id, "--messages", &n, "--json"], None)?;
        serde_json::from_str(&out).context("parsing t3 thread show --json")
    }

    fn send(&self, thread_id: &str, message: &str) -> Result<()> {
        // Message via stdin so long prompts never hit argv limits. Never --wait (D2).
        self.run(&["thread", "send", thread_id, "-", "--json"], Some(message))?;
        Ok(())
    }

    fn start(&self, a: &StartArgs) -> Result<String> {
        let mut args = vec!["thread", "start", a.project, "-", "--json"];
        if let Some(t) = a.title {
            args.extend(["--title", t]);
        }
        if let (Some(p), Some(m)) = (a.provider, a.model) {
            args.extend(["--provider", p, "--model", m]);
        }
        let out = self.run(&args, Some(a.message))?;
        let v: serde_json::Value = serde_json::from_str(&out).context("parsing t3 thread start --json")?;
        v.get("threadId")
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("t3 thread start: no threadId in {out}"))
    }
}

/// Test double: canned list/show results, records every write.
#[derive(Default)]
pub struct FakeT3 {
    pub lists: std::cell::RefCell<std::collections::HashMap<String, Result<Vec<ThreadInfo>, String>>>,
    pub messages: std::collections::HashMap<String, Vec<Message>>,
    pub calls: std::cell::RefCell<Vec<String>>,
    pub fail_send: bool,
}

impl T3 for FakeT3 {
    fn list(&self, project: &str) -> Result<Vec<ThreadInfo>> {
        match self.lists.borrow().get(project) {
            Some(Ok(v)) => Ok(v.clone()),
            Some(Err(e)) => Err(anyhow!(e.clone())),
            None => Err(anyhow!("No T3 Code project matches '{project}'")),
        }
    }
    fn show(&self, thread_id: &str, n: u32) -> Result<ThreadShow> {
        let msgs = self.messages.get(thread_id).cloned().unwrap_or_default();
        let skip = msgs.len().saturating_sub(n as usize);
        Ok(ThreadShow { thread: serde_json::json!({ "threadId": thread_id }), messages: msgs[skip..].to_vec() })
    }
    fn send(&self, thread_id: &str, message: &str) -> Result<()> {
        if self.fail_send {
            bail!("send failed");
        }
        self.calls.borrow_mut().push(format!("send {thread_id} {message}"));
        Ok(())
    }
    fn start(&self, a: &StartArgs) -> Result<String> {
        let mut calls = self.calls.borrow_mut();
        calls.push(format!(
            "start {} title={} model={}/{} {}",
            a.project,
            a.title.unwrap_or("-"),
            a.provider.unwrap_or("-"),
            a.model.unwrap_or("-"),
            a.message
        ));
        Ok(format!("new-thread-{}", calls.len()))
    }
}

impl<T: T3 + ?Sized> T3 for std::rc::Rc<T> {
    fn list(&self, project: &str) -> Result<Vec<ThreadInfo>> {
        (**self).list(project)
    }
    fn show(&self, thread_id: &str, messages: u32) -> Result<ThreadShow> {
        (**self).show(thread_id, messages)
    }
    fn send(&self, thread_id: &str, message: &str) -> Result<()> {
        (**self).send(thread_id, message)
    }
    fn start(&self, args: &StartArgs) -> Result<String> {
        (**self).start(args)
    }
}
