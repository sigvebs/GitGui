pub mod diff;
pub mod log;
pub mod patch;
pub mod stash;
pub mod status;

use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Keeps `git` from flashing a console window on Windows for every invocation.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub const DIFF_CONTEXT: u32 = 3;

pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Debug, Clone)]
pub struct GitError(pub String);

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GitError {}

pub type Result<T> = std::result::Result<T, GitError>;

fn spawnable(program: &str) -> Command {
    // `mut` is only needed on Windows, where the no-console flag is set below.
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

pub fn git_version() -> Result<String> {
    let out = spawnable("git")
        .arg("--version")
        .output()
        .map_err(|e| GitError(format!("could not run `git`: {e}. Is git installed and on PATH?")))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushOptions {
    pub remote: String,
    pub branch: String,
    pub set_upstream: bool,
    pub force_with_lease: bool,
    pub tags: bool,
}

/// Turns the two failures that are actually about *this* app's environment into
/// advice, and passes everything else through untouched.
fn explain_push_failure(output: &str) -> String {
    let lower = output.to_lowercase();
    let hint = if lower.contains("could not read username")
        || lower.contains("could not read password")
        || lower.contains("terminal prompts disabled")
    {
        Some(
            "git needed credentials but this app cannot prompt for them. \
             Configure a credential helper (`git config --global credential.helper osxkeychain` \
             on macOS, `manager` on Windows) or use an SSH remote with your key in ssh-agent.",
        )
    } else if lower.contains("permission denied (publickey")
        || lower.contains("host key verification failed")
    {
        Some(
            "SSH could not authenticate without prompting. Add your key to ssh-agent \
             (`ssh-add`), and make sure the host is already in known_hosts.",
        )
    } else if lower.contains("stale info") || lower.contains("fetch first")
        || lower.contains("non-fast-forward")
    {
        Some("The remote has commits you do not have. Fetch and merge or rebase first.")
    } else {
        None
    };
    match hint {
        Some(h) => format!("{}\n\n{h}", output.trim_end()),
        None => output.trim_end().to_string(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub root: PathBuf,
}

impl Repo {
    pub fn discover(start: &Path) -> Result<Repo> {
        let out = spawnable("git")
            .current_dir(start)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .map_err(|e| GitError(format!("could not run `git`: {e}")))?;
        if !out.status.success() {
            return Err(GitError(format!(
                "{} is not inside a git repository",
                start.display()
            )));
        }
        let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if root.is_empty() {
            return Err(GitError("git returned an empty repository root".into()));
        }
        Ok(Repo {
            root: PathBuf::from(root),
        })
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    fn base(&self) -> Command {
        let mut c = spawnable("git");
        c.current_dir(&self.root)
            .arg("--no-pager")
            // Neutralise user config that would corrupt machine-read output.
            .args(["-c", "color.ui=false"])
            .args(["-c", "core.pager=cat"])
            .args(["-c", "diff.external="])
            .args(["-c", "diff.noprefix=false"])
            .args(["-c", "diff.mnemonicPrefix=false"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        c
    }

    pub fn run_bytes(&self, args: &[&str]) -> Result<Vec<u8>> {
        let out = self
            .base()
            .args(args)
            .output()
            .map_err(|e| GitError(format!("git {}: {e}", args.join(" "))))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let msg = if err.is_empty() {
                format!("git {} failed", args.join(" "))
            } else {
                err
            };
            return Err(GitError(msg));
        }
        Ok(out.stdout)
    }

    pub fn run(&self, args: &[&str]) -> Result<String> {
        let bytes = self.run_bytes(args)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Runs git with `input` on stdin. Used for `apply` and `commit -F -`.
    pub fn run_stdin(&self, args: &[&str], input: &str) -> Result<String> {
        let mut child = self
            .base()
            .args(args)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| GitError(format!("git {}: {e}", args.join(" "))))?;
        child
            .stdin
            .as_mut()
            .ok_or_else(|| GitError("failed to open git stdin".into()))?
            .write_all(input.as_bytes())
            .map_err(|e| GitError(format!("writing to git stdin: {e}")))?;
        let out = child
            .wait_with_output()
            .map_err(|e| GitError(format!("waiting for git: {e}")))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let msg = if err.is_empty() {
                format!("git {} failed", args.join(" "))
            } else {
                err
            };
            return Err(GitError(msg));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    // ---- queries -------------------------------------------------------

    pub fn status(&self) -> Result<status::Status> {
        let raw = self.run_bytes(&[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ])?;
        Ok(status::parse(&raw))
    }

    pub fn has_head(&self) -> bool {
        self.run(&["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok()
    }

    fn diff_args<'a>(&self, extra: &[&'a str], paths: &[&'a str]) -> Vec<&'a str> {
        let mut v = vec![
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "-U3",
        ];
        v.extend_from_slice(extra);
        v.push("--");
        v.extend_from_slice(paths);
        v
    }

    /// index -> worktree
    pub fn diff_unstaged(&self, path: &str) -> Result<String> {
        self.run(&self.diff_args(&[], &[path]))
    }

    /// base -> index
    pub fn diff_staged(
        &self,
        path: &str,
        orig: Option<&str>,
        base: Option<&str>,
    ) -> Result<String> {
        let mut paths = vec![path];
        if let Some(o) = orig {
            paths.push(o);
        }
        let mut extra = vec!["--cached", "--find-renames"];
        if let Some(b) = base {
            extra.push(b);
        }
        self.run(&self.diff_args(&extra, &paths))
    }

    pub fn amend_base(&self) -> String {
        if self.run(&["rev-parse", "--verify", "--quiet", "HEAD^"]).is_ok() {
            "HEAD^".to_string()
        } else {
            EMPTY_TREE.to_string()
        }
    }

    pub fn staged_files(&self, base: &str) -> Result<Vec<log::CommitFile>> {
        let raw = self.run_bytes(&[
            "diff",
            "--cached",
            "--name-status",
            "-z",
            "--find-renames",
            base,
        ])?;
        Ok(log::parse_name_status(&raw))
    }

    // ---- history --------------------------------------------------------

    pub fn log(&self, limit: usize, all_refs: bool) -> Result<Vec<log::CommitInfo>> {
        // `git log` fails outright on an unborn branch rather than returning
        // nothing, so short-circuit.
        if !self.has_head() {
            return Ok(Vec::new());
        }
        let pretty = format!("--pretty=format:{}", log::LOG_FORMAT);
        let n = format!("-n{limit}");
        let mut args = vec![
            "log",
            "--no-color",
            "-z",
            "--date=short",
            "--topo-order",
            &pretty,
            &n,
        ];
        if all_refs {
            args.push("--all");
        }
        Ok(log::parse_log(&self.run_bytes(&args)?))
    }

    /// `-m --first-parent` matters: without it a merge commit yields no file
    /// list at all, and `--cc` output is not a diff this app can parse.
    pub fn commit_files(&self, sha: &str) -> Result<Vec<log::CommitFile>> {
        let raw = self.run_bytes(&[
            "show",
            sha,
            "--format=",
            "--name-status",
            "-z",
            "-m",
            "--first-parent",
            "--find-renames",
        ])?;
        Ok(log::parse_name_status(&raw))
    }

    pub fn commit_meta(&self, sha: &str) -> Result<log::CommitMeta> {
        let out = self.run(&[
            "show",
            "-s",
            "--date=iso",
            "--format=%an%x1f%ae%x1f%ad%x1f%D%x1f%P%x1f%B",
            sha,
        ])?;
        Ok(log::parse_meta(&out))
    }

    /// Diff of one path within a commit. Passing the pre-rename path too keeps
    /// `--find-renames` able to report it as a rename instead of an add.
    pub fn diff_commit(&self, sha: &str, path: &str, orig: Option<&str>) -> Result<String> {
        let mut args = vec![
            "show",
            sha,
            "--format=",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "-U3",
            "-m",
            "--first-parent",
            "--find-renames",
            "--",
            path,
        ];
        if let Some(o) = orig {
            args.push(o);
        }
        self.run(&args)
    }

    // ---- stashes --------------------------------------------------------

    pub fn stash_list(&self) -> Result<Vec<stash::StashEntry>> {
        if !self.has_head() {
            return Ok(Vec::new());
        }
        // No `--date` here on purpose; see STASH_FORMAT.
        let pretty = format!("--format={}", stash::STASH_FORMAT);
        let raw = self.run_bytes(&["stash", "list", "-z", &pretty])?;
        Ok(stash::parse_list(&raw))
    }

    /// Files a stash would restore. Tracked changes come from diffing the stash
    /// against its first parent; untracked ones only exist in the third parent,
    /// so they are listed separately and flagged.
    pub fn stash_files(&self, e: &stash::StashEntry) -> Result<Vec<stash::StashFile>> {
        let base = format!("{}^1", e.sha);
        let raw = self.run_bytes(&[
            "diff",
            "--name-status",
            "-z",
            "--find-renames",
            &base,
            &e.sha,
        ])?;
        let mut out: Vec<stash::StashFile> = log::parse_name_status(&raw)
            .into_iter()
            .map(|f| stash::StashFile {
                status: f.status,
                path: f.path,
                orig_path: f.orig_path,
                untracked: false,
            })
            .collect();

        if let Some(parent) = e.untracked_parent() {
            let raw = self.run_bytes(&["ls-tree", "-r", "--name-only", "-z", parent])?;
            for path in stash::parse_paths(&raw) {
                out.push(stash::StashFile {
                    status: status::Change::Untracked,
                    path,
                    orig_path: None,
                    untracked: true,
                });
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    /// Diff of one tracked path inside a stash.
    ///
    /// `git stash show -p -- <path>` cannot do this — it rejects the pathspec
    /// as a second revision — so diff the stash against its first parent.
    pub fn diff_stash(&self, sha: &str, path: &str, orig: Option<&str>) -> Result<String> {
        let base = format!("{sha}^1");
        let mut args = vec![
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "-U3",
            "--find-renames",
            &base,
            sha,
            "--",
            path,
        ];
        if let Some(o) = orig {
            args.push(o);
        }
        self.run(&args)
    }

    /// Raw bytes of an untracked file held in a stash's third parent.
    pub fn stash_untracked_blob(&self, parent: &str, path: &str) -> Result<Vec<u8>> {
        self.run_bytes(&["show", &format!("{parent}:{path}")])
    }

    pub fn stash_push(&self, message: &str, include_untracked: bool) -> Result<String> {
        let mut args = vec!["stash", "push"];
        if include_untracked {
            args.push("--include-untracked");
        }
        let msg = message.trim();
        if !msg.is_empty() {
            args.push("-m");
            args.push(msg);
        }
        self.run(&args)
    }

    pub fn stash_apply(&self, refname: &str) -> Result<String> {
        self.run(&["stash", "apply", refname])
    }

    pub fn stash_pop(&self, refname: &str) -> Result<String> {
        self.run(&["stash", "pop", refname])
    }

    pub fn stash_drop(&self, refname: &str) -> Result<String> {
        self.run(&["stash", "drop", refname])
    }

    /// Re-creates a dropped stash entry from its commit, which survives in the
    /// object store long enough for an undo to work.
    pub fn stash_store(&self, sha: &str, message: &str) -> Result<String> {
        self.run(&["stash", "store", "-m", message, sha])
    }

    pub fn last_commit_message(&self) -> Result<String> {
        self.run(&["log", "-1", "--pretty=%B"])
    }

    pub fn user_signature(&self) -> Result<String> {
        let name = self.run(&["config", "user.name"])?.trim().to_string();
        let email = self.run(&["config", "user.email"])?.trim().to_string();
        Ok(format!("Signed-off-by: {name} <{email}>"))
    }

    pub fn remotes(&self) -> Result<Vec<String>> {
        Ok(self
            .run(&["remote"])?
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect())
    }

    /// Runs `git push` and returns its combined output either way, since git
    /// reports useful detail on stderr even when it succeeds.
    ///
    /// `GIT_TERMINAL_PROMPT=0` is essential: this runs with no terminal, so
    /// without it a repository needing credentials would block forever instead
    /// of failing with something we can show the user.
    pub fn push(&self, o: &PushOptions) -> std::result::Result<String, String> {
        let mut cmd = self.base();
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        cmd.arg("push");
        cmd.arg("--verbose");
        if o.set_upstream {
            cmd.arg("--set-upstream");
        }
        if o.force_with_lease {
            // Never plain --force: this refuses if the remote moved in a way we
            // have not seen.
            cmd.arg("--force-with-lease");
        }
        if o.tags {
            cmd.arg("--tags");
        }
        cmd.arg(&o.remote);
        cmd.arg(&o.branch);

        let out = cmd
            .output()
            .map_err(|e| format!("could not run git push: {e}"))?;
        let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
        let err = String::from_utf8_lossy(&out.stderr);
        if !err.trim().is_empty() {
            if !combined.is_empty() && !combined.ends_with('\n') {
                combined.push('\n');
            }
            combined.push_str(&err);
        }
        if out.status.success() {
            Ok(combined)
        } else {
            Err(explain_push_failure(&combined))
        }
    }

    /// Name of the branch being merged in, or None when no merge is underway.
    /// `MERGE_HEAD` only exists between a conflicted merge and its commit.
    pub fn merge_head(&self) -> Option<String> {
        self.run(&["rev-parse", "--verify", "--quiet", "MERGE_HEAD"]).ok()?;
        if let Ok(name) = self.run(&["name-rev", "--name-only", "MERGE_HEAD"]) {
            let name = name.trim().to_string();
            if !name.is_empty() && name != "undefined" {
                return Some(name);
            }
        }
        self.run(&["rev-parse", "--short", "MERGE_HEAD"])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// The message git prepared for the merge commit, comments stripped.
    pub fn merge_message(&self) -> Option<String> {
        let dir = self.run(&["rev-parse", "--absolute-git-dir"]).ok()?;
        let path = Path::new(dir.trim()).join("MERGE_MSG");
        let text = std::fs::read_to_string(path).ok()?;
        let body: Vec<&str> = text
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect();
        let body = body.join("
").trim().to_string();
        if body.is_empty() {
            None
        } else {
            Some(body)
        }
    }

    pub fn abort_merge(&self) -> Result<()> {
        self.run(&["merge", "--abort"]).map(|_| ())
    }

    /// Resolves a conflict by taking one side wholesale. `ours` is the current
    /// branch, `theirs` the one being merged in.
    pub fn take_side(&self, path: &str, ours: bool) -> Result<()> {
        let side = if ours { "--ours" } else { "--theirs" };
        self.run(&["checkout", side, "--", path])?;
        self.run(&["add", "--", path]).map(|_| ())
    }

    pub fn branches(&self) -> Result<Vec<String>> {
        let out = self.run(&[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads",
        ])?;
        Ok(out.lines().map(|s| s.to_string()).collect())
    }

    // ---- mutations -----------------------------------------------------

    pub fn stage_paths(&self, paths: &[String]) -> Result<()> {
        let mut args = vec!["add", "--"];
        let refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        args.extend_from_slice(&refs);
        self.run(&args).map(|_| ())
    }

    pub fn unstage_paths(&self, paths: &[String]) -> Result<()> {
        // `restore --staged` needs a HEAD; fall back to `rm --cached` on an
        // unborn branch so the very first commit can still be un-staged.
        if self.has_head() {
            let mut args = vec!["restore", "--staged", "--"];
            let refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
            args.extend_from_slice(&refs);
            self.run(&args).map(|_| ())
        } else {
            let mut args = vec!["rm", "--cached", "--force", "--quiet", "--"];
            let refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
            args.extend_from_slice(&refs);
            self.run(&args).map(|_| ())
        }
    }

    pub fn unstage_paths_from(&self, base: &str, paths: &[String]) -> Result<()> {
        for p in paths {
            if self.run(&["cat-file", "-e", &format!("{base}:{p}")]).is_ok() {
                self.run(&["restore", "--source", base, "--staged", "--", p])?;
            } else {
                self.run(&[
                    "rm",
                    "--cached",
                    "--force",
                    "--quiet",
                    "--ignore-unmatch",
                    "--",
                    p,
                ])?;
            }
        }
        Ok(())
    }

    pub fn checkout_paths(&self, paths: &[String]) -> Result<()> {
        let mut args = vec!["checkout", "--"];
        let refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        args.extend_from_slice(&refs);
        self.run(&args).map(|_| ())
    }

    pub fn apply_patch(&self, patch: &str, mode: patch::ApplyMode, cached: bool) -> Result<()> {
        let mut args = vec!["apply", "--whitespace=nowarn", "--recount"];
        if cached {
            args.push("--cached");
        }
        if mode == patch::ApplyMode::Reverse {
            args.push("--reverse");
        }
        args.push("-");
        self.run_stdin(&args, patch).map(|_| ())
    }

    pub fn commit(&self, message: &str, amend: bool) -> Result<String> {
        let mut args = vec!["commit", "--cleanup=strip", "-F", "-"];
        if amend {
            args.push("--amend");
        }
        self.run_stdin(&args, message)
    }

    pub fn checkout_branch(&self, name: &str) -> Result<()> {
        self.run(&["checkout", name]).map(|_| ())
    }

    pub fn create_branch(&self, name: &str) -> Result<()> {
        self.run(&["checkout", "-b", name]).map(|_| ())
    }
}
