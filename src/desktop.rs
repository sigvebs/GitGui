//! Handing a file to whatever the system has registered to open it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Rewrites a path with the platform's own separator throughout.
///
/// `git rev-parse --show-toplevel` answers with forward slashes even on
/// Windows, so joining a repo-relative path onto it yields something like
/// `C:/repo\src/file.rs`. Explorer cannot parse that and silently opens the
/// user's Documents folder instead of the file.
pub fn native(path: &Path) -> PathBuf
{
	path.components().collect()
}

pub fn open(path: &Path) -> Result<(), String>
{
	let status = launcher()
		.arg(native(path))
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.status()
		.map_err(|e| format!("could not run the system file opener: {e}"))?;

	// explorer.exe reports a nonzero code even when it handed the file off
	// successfully, so its status says nothing useful.
	if !IGNORE_STATUS && !status.success()
	{
		return Err(format!("the system file opener exited with {status}"));
	}
	Ok(())
}

#[cfg(windows)]
const IGNORE_STATUS: bool = true;

#[cfg(not(windows))]
const IGNORE_STATUS: bool = false;

#[cfg(windows)]
fn launcher() -> Command
{
	let mut c = Command::new("explorer.exe");
	c.creation_flags(CREATE_NO_WINDOW);
	c
}

#[cfg(target_os = "macos")]
fn launcher() -> Command
{
	Command::new("open")
}

#[cfg(not(any(windows, target_os = "macos")))]
fn launcher() -> Command
{
	Command::new("xdg-open")
}

/// Splits a configured editor command the way a shell would, honouring both
/// quote styles. `core.editor` on Windows is commonly a single-quoted path
/// followed by flags.
pub fn split_command(command: &str) -> Vec<String>
{
	let mut out = Vec::new();
	let mut cur = String::new();
	let mut quote: Option<char> = None;
	let mut quoted = false;

	for c in command.chars()
	{
		match quote
		{
			Some(q) if c == q =>
			{
				quote = None;
			}
			Some(_) =>
			{
				cur.push(c);
			}
			None if c == '\'' || c == '\"' =>
			{
				quote = Some(c);
				quoted = true;
			}
			None if c.is_whitespace() =>
			{
				if quoted || !cur.is_empty()
				{
					out.push(std::mem::take(&mut cur));
					quoted = false;
				}
			}
			None =>
			{
				cur.push(c);
			}
		}
	}
	if quoted || !cur.is_empty()
	{
		out.push(cur);
	}
	out
}

fn program_key(program: &str) -> String
{
	let base = Path::new(program)
		.file_name()
		.map(|s| s.to_string_lossy().to_string())
		.unwrap_or_else(|| program.to_string())
		.to_lowercase();
	for ext in [".exe", ".cmd", ".bat", ".com"]
	{
		if let Some(stripped) = base.strip_suffix(ext)
		{
			return stripped.to_string();
		}
	}
	base
}

/// The full argument list for opening `path` at `line`, or None when no editor
/// command was configured. Editors disagree about how to be told a line, and an
/// unrecognised one is simply handed the file.
pub fn editor_argv(command: &str, path: &Path, line: u32) -> Option<Vec<String>>
{
	let mut argv = split_command(command);
	if argv.is_empty()
	{
		return None;
	}
	let file = native(path).to_string_lossy().into_owned();
	let key = program_key(&argv[0]);

	match key.as_str()
	{
		"code" | "code-insiders" | "codium" | "vscodium" =>
		{
			argv.push("-g".to_string());
			argv.push(format!("{file}:{line}"));
		}
		"notepad++" =>
		{
			argv.push(format!("-n{line}"));
			argv.push(file);
		}
		"subl" | "sublime_text" =>
		{
			argv.push(format!("{file}:{line}"));
		}
		"vim" | "nvim" | "gvim" | "vi" | "nano" | "emacs" | "kate" | "gedit" =>
		{
			argv.push(format!("+{line}"));
			argv.push(file);
		}
		"clion" | "clion64" | "idea" | "idea64" | "pycharm" | "rider" | "webstorm"
		| "rustrover" | "goland" =>
		{
			argv.push("--line".to_string());
			argv.push(line.to_string());
			argv.push(file);
		}
		_ =>
		{
			argv.push(file);
		}
	}
	Some(argv)
}

/// Launches a configured editor on one line of a file.
pub fn open_in_editor(command: &str, path: &Path, line: u32) -> Result<(), String>
{
	let argv = editor_argv(command, path, line)
		.ok_or_else(|| "the configured editor command is empty".to_string())?;
	let mut cmd = Command::new(&argv[0]);
	#[cfg(windows)]
	cmd.creation_flags(CREATE_NO_WINDOW);
	cmd.args(&argv[1..])
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null());
	cmd.spawn()
		.map(|_| ())
		.map_err(|e| format!("could not run {}: {e}", argv[0]))
}

#[cfg(test)]
mod tests
{
	use super::*;

	#[test]
	fn splits_a_single_quoted_program_with_flags()
	{
		let cmd = "'C:/Program Files/Notepad++/notepad++.exe' -multiInst -nosession";
		let argv = split_command(cmd);
		assert_eq!(
			argv,
			vec![
				"C:/Program Files/Notepad++/notepad++.exe",
				"-multiInst",
				"-nosession"
			]
		);
	}

	#[test]
	fn splits_a_double_quoted_program()
	{
		let argv = split_command("\"C:\\tools\\my editor.exe\" --wait");
		assert_eq!(argv, vec!["C:\\tools\\my editor.exe", "--wait"]);
	}

	#[test]
	fn an_empty_command_splits_to_nothing()
	{
		assert!(split_command("   ").is_empty());
		assert!(editor_argv("", Path::new("/tmp/f.rs"), 4).is_none());
	}

	#[test]
	fn notepad_plus_plus_gets_its_own_line_flag()
	{
		let cmd = "'C:/Program Files/Notepad++/notepad++.exe' -multiInst";
		let argv = editor_argv(cmd, Path::new("C:/repo/src/main.rs"), 42).unwrap();
		assert_eq!(argv[0], "C:/Program Files/Notepad++/notepad++.exe");
		assert_eq!(argv[1], "-multiInst", "the configured flags are kept");
		assert_eq!(argv[2], "-n42");
		assert!(argv[3].ends_with("main.rs"));
		assert!(
			!argv[3].contains('/') || !cfg!(windows),
			"the path handed over must be native: {:?}",
			argv[3]
		);
	}

	#[test]
	fn vscode_uses_goto_syntax()
	{
		let argv = editor_argv("code", Path::new("/repo/src/main.rs"), 7).unwrap();
		assert_eq!(argv[0], "code");
		assert_eq!(argv[1], "-g");
		assert!(argv[2].ends_with("main.rs:7"), "got {:?}", argv[2]);
	}

	#[test]
	fn vi_style_editors_use_a_plus_argument()
	{
		let argv = editor_argv("vim", Path::new("/repo/f.txt"), 12).unwrap();
		assert_eq!(argv[1], "+12");
		assert!(argv[2].ends_with("f.txt"));
	}

	#[test]
	fn jetbrains_editors_use_the_line_switch()
	{
		let argv = editor_argv("clion64.exe", Path::new("/repo/f.rs"), 99).unwrap();
		assert_eq!(argv[1], "--line");
		assert_eq!(argv[2], "99");
	}

	#[test]
	fn an_unknown_editor_is_just_handed_the_file()
	{
		let argv = editor_argv("mystery-editor", Path::new("/repo/f.rs"), 5).unwrap();
		assert_eq!(argv.len(), 2, "no invented line flag: {argv:?}");
		assert!(argv[1].ends_with("f.rs"));
	}

	#[test]
	fn native_rewrites_mixed_separators()
	{
		let out = native(Path::new("C:/repo/root/src/file.rs"));
		let text = out.to_string_lossy().into_owned();
		if cfg!(windows)
		{
			assert_eq!(text, "C:\\repo\\root\\src\\file.rs");
			assert!(!text.contains('/'), "still mixed: {text}");
		}
		else
		{
			assert_eq!(text, "C:/repo/root/src/file.rs");
		}
	}

	#[test]
	fn native_leaves_a_clean_path_alone()
	{
		let clean = if cfg!(windows)
		{
			"C:\\repo\\src\\file.rs"
		}
		else
		{
			"/repo/src/file.rs"
		};
		assert_eq!(native(Path::new(clean)).to_string_lossy(), clean);
	}
}
