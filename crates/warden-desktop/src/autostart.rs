//! Start with Windows: a value under the user's `Run` key that starts Warden with `--autostart` at sign-in.
//! Turning it off removes the value again, so nothing is left behind.

use std::io;
use std::path::Path;
use windows_registry::CURRENT_USER;

/// Where Windows looks for programs to start when the user signs in.
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// The name of Warden's value under it.
pub const VALUE_NAME: &str = "Warden";
/// The argument that tells Warden it was started by Windows.
pub const AUTOSTART_ARG: &str = "--autostart";

/// One registry value that starts a command at sign-in.
pub struct RunValue {
    key: String,
    name: String,
    command: String,
}

/// The command line Windows runs: the app with the options it was started with (its data folder, so a
/// separate Warden starts as itself) and `--autostart`. Every part is quoted, so paths with spaces work.
pub fn command_for(exe: &Path, options: &[String]) -> String {
    // The program's path is read up to the next quote, without escapes (and a path holds no quote).
    let mut parts = vec![format!("\"{}\"", exe.display())];
    parts.extend(options.iter().map(|o| quote_arg(o)));
    parts.push(AUTOSTART_ARG.to_owned());
    parts.join(" ")
}

/// One argument quoted the way Windows programs split their command line: backslashes count as escapes only before
/// a quote, so those (and the ones before the closing quote, as in a folder ending in `\`) are doubled.
fn quote_arg(arg: &str) -> String {
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            // The backslashes already written, as many again, and one for the quote itself.
            '"' => {
                out.push_str(&"\\".repeat(backslashes + 1));
                backslashes = 0;
            }
            _ => backslashes = 0,
        }
        out.push(c);
    }
    // Doubled before the closing quote.
    out.push_str(&"\\".repeat(backslashes));
    out.push('"');
    out
}

impl RunValue {
    /// Warden's own value, starting the program that is running now with these options.
    pub fn for_this_app(options: &[String]) -> io::Result<RunValue> {
        Ok(RunValue::new(
            RUN_KEY,
            VALUE_NAME,
            &command_for(&std::env::current_exe()?, options),
        ))
    }

    pub fn new(key: &str, name: &str, command: &str) -> RunValue {
        RunValue {
            key: key.to_owned(),
            name: name.to_owned(),
            command: command.to_owned(),
        }
    }

    /// Set, and set to this program (a value left by Warden installed elsewhere does not count).
    pub fn is_set(&self) -> bool {
        CURRENT_USER
            .open(&self.key)
            .and_then(|k| k.get_string(&self.name))
            .is_ok_and(|v| v == self.command)
    }

    pub fn set(&self, on: bool) -> io::Result<()> {
        if on {
            return Ok(CURRENT_USER
                .create(&self.key)?
                .set_string(&self.name, &self.command)?);
        }
        let Ok(key) = CURRENT_USER.options().read().write().open(&self.key) else {
            return Ok(());
        };
        match key.remove_value(&self.name) {
            Ok(()) => Ok(()),
            Err(e) if io::Error::from(e.clone()).kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_is_quoted_and_marked() {
        assert_eq!(
            command_for(
                Path::new(r"%LOCALAPPDATA%\My Programs\Warden\Warden.exe"),
                &[]
            ),
            r#""%LOCALAPPDATA%\My Programs\Warden\Warden.exe" --autostart"#
        );
        let options = ["--data-dir".to_owned(), r"%TEMP%\Other Warden".to_owned()];
        assert_eq!(
            command_for(Path::new(r"%LOCALAPPDATA%\Warden\Warden.exe"), &options),
            r#""%LOCALAPPDATA%\Warden\Warden.exe" "--data-dir" "%TEMP%\Other Warden" --autostart"#
        );
    }

    /// Splits arguments as Windows programs do (after the program's path): 2n backslashes before a quote are n and
    /// the quote toggles quoting; 2n+1 are n and a literal quote; other backslashes are literal.
    fn split(line: &str) -> Vec<String> {
        let (mut args, mut cur, mut quoted, mut any) = (Vec::new(), String::new(), false, false);
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' {
                let start = i;
                while i < chars.len() && chars[i] == '\\' {
                    i += 1;
                }
                let n = i - start;
                if chars.get(i) == Some(&'"') {
                    cur.push_str(&"\\".repeat(n / 2));
                    if n % 2 == 1 {
                        cur.push('"');
                        i += 1;
                    }
                } else {
                    cur.push_str(&"\\".repeat(n));
                }
                any = true;
                continue;
            }
            match c {
                '"' => {
                    quoted = !quoted;
                    any = true;
                }
                ' ' if !quoted => {
                    if any {
                        args.push(std::mem::take(&mut cur));
                    }
                    any = false;
                }
                _ => {
                    cur.push(c);
                    any = true;
                }
            }
            i += 1;
        }
        if any {
            args.push(cur);
        }
        args
    }

    #[test]
    fn options_survive_the_command_line() {
        let options: Vec<String> = [
            "--data-dir",
            r"%TEMP%\Warden data\",
            r"%TEMP%\odd\\",
            r#"a "quoted" \"name"#,
            "",
        ]
        .map(String::from)
        .to_vec();
        let line = command_for(Path::new(r"%LOCALAPPDATA%\Warden\Warden.exe"), &options);
        let after_exe = line
            .strip_prefix(r#""%LOCALAPPDATA%\Warden\Warden.exe" "#)
            .expect("program first");
        let mut want = options.clone();
        want.push(AUTOSTART_ARG.to_owned());
        assert_eq!(split(after_exe), want, "{line}");
    }

    /// The real registry code, on a throwaway key instead of the `Run` key, so nothing starts at sign-in. The key
    /// is removed afterwards, even when the test fails.
    #[test]
    fn switching_on_and_off_leaves_nothing_behind() {
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = CURRENT_USER.remove_tree(&self.0);
            }
        }
        let root = format!(
            r"Software\Warden-test-{}-{}",
            std::process::id(),
            warden_core::time::now_ms()
        );
        let _cleanup = Cleanup(root.clone());
        let key = format!(r"{root}\Run");
        let command = r#""%LOCALAPPDATA%\Warden\Warden.exe" --autostart"#;
        let value = RunValue::new(&key, VALUE_NAME, command);

        assert!(!value.is_set());
        value.set(false).expect("off when already off");
        value.set(true).expect("on");
        assert!(value.is_set());
        assert_eq!(
            CURRENT_USER
                .open(&key)
                .and_then(|k| k.get_string(VALUE_NAME))
                .expect("value"),
            command
        );
        // Another program's value under the same name does not count as on.
        let other = RunValue::new(&key, VALUE_NAME, r#""%TEMP%\Old\Warden.exe" --autostart"#);
        assert!(!other.is_set());

        value.set(false).expect("off");
        assert!(!value.is_set());
        assert!(
            CURRENT_USER
                .open(&key)
                .and_then(|k| k.get_string(VALUE_NAME))
                .is_err(),
            "the value is gone"
        );
        value.set(false).expect("off twice");
    }
}
