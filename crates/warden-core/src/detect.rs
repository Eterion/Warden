//! Finding things on this computer without assuming whose it is: account folders in the home folder, the CLIs on
//! PATH, the desktop apps. Everything is read from the environment Warden runs in ([`Env`]), so tests use fixtures.

use crate::model::{Identity, Provider};
use crate::{claude, codex};
use serde::Serialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The parts of the environment detection depends on.
#[derive(Debug, Clone, Default)]
pub struct Env {
    #[cfg(debug_assertions)]
    pub no_hooks: bool,
    pub home: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    pub path: Vec<PathBuf>,
    /// `CLAUDE_CONFIG_DIR` and `CODEX_HOME`, when set.
    pub claude_config_dir: Option<PathBuf>,
    pub codex_home: Option<PathBuf>,
    /// `CLAUDE_CODE_GIT_BASH_PATH`, when set: the Git Bash Claude Code runs hooks in.
    pub claude_git_bash: Option<PathBuf>,
    /// The folders Claude Code looks in for Git Bash before PATH ([`claude_program_files`]).
    pub program_files: Vec<PathBuf>,
}

/// Where Claude Code looks for Git Bash before PATH: `Git\bin\bash.exe` in these two folders on the C: drive,
/// whatever drive Windows is on and wherever `%ProgramFiles%` points (Claude Code's own fixed list).
pub fn claude_program_files() -> Vec<PathBuf> {
    let mut root = std::ffi::OsString::from("C:");
    root.push(std::path::MAIN_SEPARATOR_STR);
    let root = PathBuf::from(root);
    vec![root.join("Program Files"), root.join("Program Files (x86)")]
}

impl Env {
    pub fn from_process() -> Env {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Env {
            #[cfg(debug_assertions)]
            no_hooks: false,
            home: var("USERPROFILE").or_else(|| var("HOME")),
            local_app_data: var("LOCALAPPDATA"),
            path: std::env::var_os("PATH")
                .map(|p| std::env::split_paths(&p).collect())
                .unwrap_or_default(),
            claude_config_dir: var("CLAUDE_CONFIG_DIR"),
            codex_home: var("CODEX_HOME"),
            claude_git_bash: var("CLAUDE_CODE_GIT_BASH_PATH"),
            program_files: claude_program_files(),
        }
    }

    pub fn hooks_off(&self) -> bool {
        #[cfg(debug_assertions)]
        {
            self.no_hooks
        }
        #[cfg(not(debug_assertions))]
        {
            false
        }
    }

    /// The desktop app's default account folder (`~/.claude`, `~/.codex`).
    pub fn default_folder(&self, provider: Provider) -> Option<PathBuf> {
        self.home
            .as_ref()
            .map(|h| h.join(provider.default_folder_name()))
    }

    /// Whether `folder` is the desktop app's default folder, which the desktop app uses. Both are compared as the
    /// real folders they are ([`same_folder`]), so no other way of writing it slips past.
    pub fn is_desktop_folder(&self, provider: Provider, folder: &Path) -> bool {
        self.default_folder(provider)
            .is_some_and(|d| same_folder(&d, folder))
    }

    /// The desktop app whose default folder `folder` is, whichever provider it is added as.
    pub fn desktop_app_of(&self, folder: &Path) -> Option<Provider> {
        let real = real_folder(folder);
        [Provider::Claude, Provider::Codex].into_iter().find(|&p| {
            self.default_folder(p)
                .is_some_and(|d| same_path(&real_folder(&d), &real))
        })
    }

    /// Refuse the Codex app's folder and everything below it before probing any account files.
    pub fn in_codex_app(&self, folder: &Path) -> bool {
        let Some(root) = self.default_folder(Provider::Codex) else {
            return false;
        };
        let root = real_folder(&root);
        real_folder(folder).ancestors().any(|p| same_path(p, &root))
    }
}

/// A folder as it really is: spaces and quotes around it dropped, then, when it can be opened, resolved by Windows
/// (`.` and `..`, 8.3 short names, links and junctions, the case on disk). A folder that cannot be opened keeps its
/// written form with `.` and `..` taken out.
pub fn real_folder(folder: &Path) -> PathBuf {
    let cleaned = match folder.to_str() {
        Some(text) => PathBuf::from(text.trim().trim_matches('"').trim()),
        None => folder.to_owned(),
    };
    match fs::canonicalize(&cleaned) {
        Ok(real) => without_verbatim_prefix(real),
        Err(_) => lexically_normal(&cleaned),
    }
}

/// Whether two folders are the same real folder ([`real_folder`]), compared as Windows does ([`same_path`]).
pub fn same_folder(a: &Path, b: &Path) -> bool {
    same_path(&real_folder(a), &real_folder(b))
}

/// The form [`fs::canonicalize`] gives on Windows without its verbatim prefix (`\\?\` before a drive, `\\?\UNC\`
/// before a network share, which goes back to its usual two backslashes).
pub(crate) fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

/// The path with each `.` taken out and each `..` taking out the part before it.
fn lexically_normal(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            _ => out.push(part),
        }
    }
    out
}

/// Compares two paths as Windows does: without case, and ignoring a trailing separator.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    norm(a) == norm(b)
}

/// Whether the folder looks like an account folder of this provider.
pub fn looks_like_account(provider: Provider, folder: &Path, home: Option<&Path>) -> bool {
    match provider {
        Provider::Claude => claude::looks_like_account(folder, home),
        Provider::Codex => codex::looks_like_account(folder),
    }
}

/// Whether the folder holds a login (without opening it).
pub fn has_login(provider: Provider, folder: &Path) -> bool {
    match provider {
        Provider::Claude => claude::has_login(folder),
        Provider::Codex => codex::has_login(folder),
    }
}

/// Who the folder is signed in as. `Ok(None)`: signed out.
pub fn read_identity(
    provider: Provider,
    folder: &Path,
    home: Option<&Path>,
) -> io::Result<Option<Identity>> {
    match provider {
        Provider::Claude => claude::read_identity(folder, home),
        Provider::Codex => codex::read_identity_from_login_file(folder),
    }
}

/// The folder in the home that holds Warden's own account folders (`claude-<name>`, `codex-<name>`), and nothing else.
pub const WARDEN_FOLDER: &str = ".warden";

/// The provider whose account folder a name stands for, `<prefix><provider><then>…` ignoring case: `.claude…` in the
/// home (prefix `.`), `claude-…` in `~/.warden` (then `-`).
fn provider_of(name: &str, prefix: &str, then: &str) -> Option<Provider> {
    let name = name.to_lowercase();
    let rest = name.strip_prefix(prefix)?;
    [Provider::Claude, Provider::Codex].into_iter().find(|p| {
        rest.strip_prefix(&p.default_folder_name()[1..])
            .is_some_and(|after| after.starts_with(then))
    })
}

/// The folders in `dir` named `<prefix><provider><then>…`, in name order.
fn folders_named(dir: &Path, prefix: &str, then: &str) -> Vec<(Provider, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(Provider, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            Some((
                provider_of(&e.file_name().to_string_lossy(), prefix, then)?,
                e.path(),
            ))
        })
        .collect();
    found.sort_by(|a, b| a.1.cmp(&b.1));
    found
}

/// Warden account candidates from environment overrides, Warden's own folders (`~/.warden/claude-*`,
/// `~/.warden/codex-*`) and account folders in the home (`~/.claude-*`, `~/.codex-*`).
/// Neither desktop default is a candidate, nor `~/.warden` itself; Codex account files are never probed under its default.
pub fn account_folders(env: &Env) -> Vec<(Provider, PathBuf)> {
    account_folders_except(env, None)
}

pub fn account_folders_except(env: &Env, own_sign_in: Option<&Path>) -> Vec<(Provider, PathBuf)> {
    let mut found: Vec<(Provider, PathBuf)> = Vec::new();
    let warden = env.home.as_ref().map(|h| h.join(WARDEN_FOLDER));
    let mut add = |provider: Provider, folder: PathBuf| {
        if !own_sign_in.is_some_and(|own| same_folder(own, &folder))
            && !warden.as_ref().is_some_and(|w| same_folder(w, &folder))
            && !env.in_codex_app(&folder)
            && env.desktop_app_of(&folder) != Some(Provider::Claude)
            && looks_like_account(provider, &folder, env.home.as_deref())
            && !found.iter().any(|(_, f)| same_path(f, &folder))
        {
            found.push((provider, folder));
        }
    };
    if let Some(d) = env.claude_config_dir.clone() {
        add(Provider::Claude, d);
    }
    if let Some(d) = env.codex_home.clone() {
        add(Provider::Codex, d);
    }
    if let Some(home) = &env.home {
        // `~/.warden` may be a link the user made: a folder Warden leaves alone is never listed.
        let own = match &warden {
            Some(w)
                if !env.in_codex_app(w)
                    && env.desktop_app_of(w).is_none()
                    && !own_sign_in.is_some_and(|o| same_folder(o, w)) =>
            {
                folders_named(w, "", "-")
            }
            _ => Vec::new(),
        };
        for (p, f) in own.into_iter().chain(folders_named(home, ".", "")) {
            add(p, f);
        }
    }
    found
}

/// A label for a folder: "Claude" or "Codex" for the default folders and folders not named after the provider,
/// otherwise from the folder's name (`.claude-second` and `.warden/claude-second` give "Claude second").
pub fn suggested_label(provider: Provider, folder: &Path) -> String {
    let name = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = name.trim_start_matches('.');
    let named = &provider.default_folder_name()[1..];
    let rest = name
        .get(..named.len())
        .filter(|start| start.eq_ignore_ascii_case(named))
        .and_then(|_| name.get(named.len()..))
        .unwrap_or("")
        .trim_matches(|c: char| c == '-' || c == '_' || c == '.' || c.is_whitespace())
        .replace(['-', '_'], " ");
    if rest.is_empty() {
        provider.name().to_owned()
    } else {
        format!("{} {rest}", provider.name())
    }
}

/// The folder a new account gets: `~/.warden/claude-<name>` or `~/.warden/codex-<name>`, made from the label, and
/// not yet taken. Creating it creates `~/.warden` too when it is missing.
pub fn new_account_folder(env: &Env, provider: Provider, label: &str) -> Option<PathBuf> {
    let parent = env.home.as_ref()?.join(WARDEN_FOLDER);
    let mut slug: String = label
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-');
    let prefix = &provider.default_folder_name()[1..];
    let slug = slug.strip_prefix(prefix).unwrap_or(slug).trim_matches('-');
    let base = if slug.is_empty() {
        format!("{prefix}-account")
    } else {
        format!("{prefix}-{slug}")
    };
    (0..100)
        .map(|i| {
            parent.join(if i == 0 {
                base.clone()
            } else {
                format!("{base}-{}", i + 1)
            })
        })
        .find(|p| !p.exists())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolKind {
    App,
    Cli,
}

/// A desktop app or command-line tool Warden found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub provider: Provider,
    pub kind: ToolKind,
    pub name: String,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
}

/// Finds a program on PATH by its Windows names (`.exe` first, then `.cmd`), as a shell would.
pub fn find_on_path(env: &Env, program: &str) -> Option<PathBuf> {
    env.path.iter().find_map(|dir| {
        ["exe", "cmd"]
            .iter()
            .map(|ext| dir.join(format!("{program}.{ext}")))
            .find(|p| p.is_file())
    })
}

/// The CLI Warden runs for a provider: the one on PATH.
pub fn cli_path(env: &Env, provider: Provider) -> Option<PathBuf> {
    find_on_path(env, &provider.name().to_lowercase())
}

/// The desktop apps and CLIs. Only folders and file names are looked at; nothing is started.
pub fn tools(env: &Env) -> Vec<Tool> {
    let app = |provider: Provider, found: Option<(PathBuf, Option<String>)>| Tool {
        provider,
        kind: ToolKind::App,
        name: provider.name().to_owned(),
        version: found.as_ref().and_then(|f| f.1.clone()),
        path: found.map(|f| f.0),
    };
    let cli = |provider: Provider| {
        let path = cli_path(env, provider);
        Tool {
            provider,
            kind: ToolKind::Cli,
            name: provider.name().to_lowercase(),
            version: path.as_deref().and_then(|p| cli_version(provider, p)),
            path,
        }
    };
    vec![
        app(Provider::Claude, claude_app(env)),
        cli(Provider::Claude),
        app(Provider::Codex, codex_app(env)),
        cli(Provider::Codex),
    ]
}

/// Claude's desktop app: the per-user install (`%LOCALAPPDATA%\AnthropicClaude\app-<version>`) or its Store package.
fn claude_app(env: &Env) -> Option<(PathBuf, Option<String>)> {
    let local = env.local_app_data.as_ref()?;
    let install = local.join("AnthropicClaude");
    if install.is_dir() {
        let version = fs::read_dir(&install)
            .ok()?
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_string_lossy()
                    .strip_prefix("app-")
                    .map(str::to_owned)
            })
            .max_by(|a, b| compare_versions(a, b));
        return Some((install, version));
    }
    store_package(local, &["claude"])
}

/// Codex's desktop app: its Store package.
fn codex_app(env: &Env) -> Option<(PathBuf, Option<String>)> {
    store_package(env.local_app_data.as_ref()?, &["openai.codex", "codex"])
}

/// A Store (MSIX) package's data folder whose name starts with one of `prefixes`. The version is not in that name.
fn store_package(local: &Path, prefixes: &[&str]) -> Option<(PathBuf, Option<String>)> {
    let packages = fs::read_dir(local.join("Packages")).ok()?;
    packages.flatten().find_map(|e| {
        let name = e.file_name().to_string_lossy().to_lowercase();
        let family = name.split('_').next().unwrap_or("");
        prefixes
            .iter()
            .any(|p| family == *p || family.ends_with(&format!(".{p}")))
            .then(|| (e.path(), None))
    })
}

/// A CLI's version, read from the files beside it (nothing is started): an npm install's `package.json`
/// (`<prefix>\claude.cmd` beside `<prefix>\node_modules\@anthropic-ai\claude-code`, Codex's `@openai\codex`), or
/// Claude's native install, whose `<root>\bin\claude.exe` keeps its versions in `<root>\share\claude\versions`.
pub fn cli_version(provider: Provider, cli: &Path) -> Option<String> {
    let dir = cli.parent()?;
    let package = match provider {
        Provider::Claude => ["@anthropic-ai", "claude-code"],
        Provider::Codex => ["@openai", "codex"],
    };
    let npm = dir
        .join("node_modules")
        .join(package[0])
        .join(package[1])
        .join("package.json");
    if let Some(v) = fs::read(&npm)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["version"].as_str().map(str::to_owned))
    {
        return Some(v);
    }
    if provider != Provider::Claude {
        return None;
    }
    let versions = dir.parent()?.join("share").join("claude").join("versions");
    fs::read_dir(versions)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| looks_like_version(n))
        .max_by(|a, b| compare_versions(a, b))
}

fn looks_like_version(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() >= 2
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether `version` (`2.1.139`, perhaps with a suffix such as ` (Claude Code)` or `-beta`) is `min` or later.
pub fn version_at_least(version: &str, min: &str) -> bool {
    let v = version.split_whitespace().next().unwrap_or(version);
    let v = v.split(['-', '+']).next().unwrap_or(v);
    compare_versions(v, min) != std::cmp::Ordering::Less
}

fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |s: &str| {
        s.split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    parts(a).cmp(&parts(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::Dir;

    fn env(home: &Path) -> Env {
        Env {
            home: Some(home.to_owned()),
            local_app_data: Some(home.join("AppData/Local")),
            ..Env::default()
        }
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, "{}").expect("write");
    }

    #[test]
    fn the_desktop_folder_is_known_however_it_is_written() {
        let home = Dir::new("home");
        let h = home.path();
        fs::create_dir_all(h.join(".claude")).expect("mkdir");
        fs::create_dir_all(h.join(".claude-work")).expect("mkdir");
        let e = env(h);
        let text = h.to_string_lossy().into_owned();
        for written in [
            format!(r"{text}\.claude"),
            format!(r"{text}\.claude\"),
            format!(r"{text}/.claude"),
            format!(r#""{text}\.claude""#),
            format!(r#"  "{text}\.claude"  "#),
            format!(r"{text}\.claude-work\..\.claude"),
            format!(r"{text}\.\.claude"),
            format!(r"{}\.CLAUDE", text.to_uppercase()),
        ] {
            assert!(
                e.is_desktop_folder(Provider::Claude, Path::new(&written)),
                "{written}"
            );
            assert_eq!(
                e.desktop_app_of(Path::new(&written)),
                Some(Provider::Claude),
                "{written}"
            );
        }
        assert!(!e.is_desktop_folder(Provider::Claude, &h.join(".claude-work")));
        assert!(!e.is_desktop_folder(Provider::Codex, &h.join(".claude")));
        assert_eq!(e.desktop_app_of(&h.join(".claude-work")), None);
        // A folder that does not exist yet is compared as written, `..` taken out.
        assert!(e.is_desktop_folder(Provider::Codex, &h.join(r"gone\..\.codex")));
    }

    #[test]
    fn finds_default_and_extra_account_folders() {
        let home = Dir::new("home");
        let h = home.path();
        touch(&h.join(".claude/.credentials.json"));
        touch(&h.join(".claude-second/.credentials.json"));
        touch(&h.join(".codex/auth.json"));
        touch(&h.join(".codex-spare/config.toml"));
        fs::create_dir_all(h.join(".claude-empty")).expect("mkdir");
        fs::create_dir_all(h.join("projects")).expect("mkdir");
        let found = account_folders(&env(h));
        let names: Vec<_> = found
            .iter()
            .map(|(p, f)| (*p, f.file_name().map(|n| n.to_string_lossy().into_owned())))
            .collect();
        assert_eq!(
            names,
            vec![
                (Provider::Claude, Some(".claude-second".into())),
                (Provider::Codex, Some(".codex-spare".into())),
            ]
        );
    }

    #[test]
    fn finds_wardens_own_folders_first_then_the_homes() {
        let home = Dir::new("home");
        let h = home.path();
        touch(&h.join(".claude/.credentials.json"));
        touch(&h.join(".codex/auth.json"));
        touch(&h.join(".claude-old/.credentials.json"));
        touch(&h.join(".warden/codex-spare/config.toml"));
        touch(&h.join(".warden/claude-second/.credentials.json"));
        touch(&h.join(".warden/CLAUDE-Upper/.credentials.json"));
        // Not named `claude-*` or `codex-*`, not an account, or not a folder: never listed.
        touch(&h.join(".warden/claude/.credentials.json"));
        touch(&h.join(".warden/notes/.credentials.json"));
        touch(&h.join(".warden/.claude-dotted/.credentials.json"));
        fs::create_dir_all(h.join(".warden/codex-empty")).expect("mkdir");
        touch(&h.join(".warden/claude-file"));
        let found = account_folders(&env(h));
        assert_eq!(
            found,
            vec![
                (Provider::Claude, h.join(".warden").join("CLAUDE-Upper")),
                (Provider::Claude, h.join(".warden").join("claude-second")),
                (Provider::Codex, h.join(".warden").join("codex-spare")),
                (Provider::Claude, h.join(".claude-old")),
            ]
        );
    }

    #[test]
    fn the_warden_folder_itself_is_never_an_account() {
        let home = Dir::new("home");
        let h = home.path();
        // Even when it looks like one and an override names it.
        touch(&h.join(".warden/.credentials.json"));
        touch(&h.join(".warden/config.toml"));
        let e = Env {
            claude_config_dir: Some(h.join(".warden")),
            codex_home: Some(h.join(r".warden.")),
            ..env(h)
        };
        assert!(account_folders(&e).is_empty(), "{:?}", account_folders(&e));
        // Without a `~/.warden` at all, nothing breaks.
        assert!(account_folders(&env(&h.join("elsewhere"))).is_empty());
    }

    #[test]
    fn main_is_not_a_candidate_even_with_only_the_home_identity_file() {
        let home = Dir::new("home");
        fs::create_dir_all(home.path().join(".claude")).expect("mkdir");
        touch(&home.path().join(".claude.json"));
        assert!(account_folders(&env(home.path())).is_empty());
    }

    #[test]
    fn labels_from_folder_names() {
        assert_eq!(
            suggested_label(Provider::Claude, Path::new("x/.claude")),
            "Claude"
        );
        assert_eq!(
            suggested_label(Provider::Claude, Path::new("x/.claude-second")),
            "Claude second"
        );
        assert_eq!(
            suggested_label(Provider::Codex, Path::new("x/.codex_spare_one")),
            "Codex spare one"
        );
        assert_eq!(
            suggested_label(Provider::Codex, Path::new("x/work")),
            "Codex"
        );
        // Warden's own folders, without the dot.
        assert_eq!(
            suggested_label(Provider::Claude, Path::new("x/.warden/claude-second")),
            "Claude second"
        );
        assert_eq!(
            suggested_label(Provider::Codex, Path::new("x/.warden/Codex-Spare")),
            "Codex Spare"
        );
        // A name that is not the provider's gives the provider's name alone.
        assert_eq!(
            suggested_label(Provider::Codex, Path::new("x/workspace-two")),
            "Codex"
        );
    }

    #[test]
    fn new_folders_are_named_after_the_label_and_never_taken() {
        let home = Dir::new("home");
        let e = env(home.path());
        let own = home.path().join(".warden");
        // No `~/.warden` yet: the path is still given (creating the folder creates it).
        assert!(!own.exists());
        let first = new_account_folder(&e, Provider::Claude, "Claude 3").expect("folder");
        assert_eq!(first, own.join("claude-3"));
        fs::create_dir_all(&first).expect("mkdir");
        assert_eq!(
            new_account_folder(&e, Provider::Claude, "Claude 3"),
            Some(own.join("claude-3-2"))
        );
        assert_eq!(
            new_account_folder(&e, Provider::Codex, "Work / spare!"),
            Some(own.join("codex-work-spare"))
        );
        assert_eq!(
            new_account_folder(&e, Provider::Codex, "Codex second"),
            Some(own.join("codex-second"))
        );
        assert_eq!(
            new_account_folder(&e, Provider::Codex, "***"),
            Some(own.join("codex-account"))
        );
        // An old folder in the home with the same name does not take the name.
        fs::create_dir_all(home.path().join(".codex-second")).expect("mkdir");
        assert_eq!(
            new_account_folder(&e, Provider::Codex, "Codex second"),
            Some(own.join("codex-second"))
        );
        assert_eq!(
            new_account_folder(&Env::default(), Provider::Codex, "x"),
            None
        );
    }

    #[test]
    fn finds_tools_without_starting_them() {
        let home = Dir::new("home");
        let h = home.path();
        let bin = h.join("bin");
        touch(&bin.join("claude.exe"));
        touch(&bin.join("codex.cmd"));
        fs::create_dir_all(h.join("AppData/Local/AnthropicClaude/app-1.2.10")).expect("mkdir");
        fs::create_dir_all(h.join("AppData/Local/AnthropicClaude/app-1.2.9")).expect("mkdir");
        fs::create_dir_all(h.join("AppData/Local/Packages/OpenAI.Codex_2p2nqsd0c76g0"))
            .expect("mkdir");
        let mut e = env(h);
        e.path = vec![h.join("nothing"), bin.clone()];
        let t = tools(&e);
        assert_eq!(t[0].version.as_deref(), Some("1.2.10"));
        assert_eq!(t[1].path.as_deref(), Some(bin.join("claude.exe").as_path()));
        assert!(t[2].path.is_some(), "Codex app found");
        assert_eq!(t[3].path.as_deref(), Some(bin.join("codex.cmd").as_path()));
    }

    #[test]
    fn reads_a_cli_version_from_the_files_beside_it() {
        let home = Dir::new("cli-versions");
        let h = home.path();
        // npm: the package beside the shim.
        let npm = h.join("AppData/Roaming/npm");
        touch(&npm.join("claude.cmd"));
        let package = npm.join("node_modules/@anthropic-ai/claude-code/package.json");
        touch(&package);
        fs::write(
            &package,
            r#"{"name":"@anthropic-ai/claude-code","version":"2.1.140"}"#,
        )
        .expect("write");
        assert_eq!(
            cli_version(Provider::Claude, &npm.join("claude.cmd")).as_deref(),
            Some("2.1.140")
        );
        assert_eq!(cli_version(Provider::Codex, &npm.join("codex.cmd")), None);
        // Native: the newest of the versions kept beside bin.
        let bin = h.join(".local/bin");
        touch(&bin.join("claude.exe"));
        for v in ["2.1.9", "2.1.138", "notes"] {
            touch(&h.join(".local/share/claude/versions").join(v));
        }
        assert_eq!(
            cli_version(Provider::Claude, &bin.join("claude.exe")).as_deref(),
            Some("2.1.138")
        );
        assert_eq!(
            cli_version(Provider::Claude, &h.join("elsewhere/claude.exe")),
            None
        );
    }

    #[test]
    fn compares_versions_by_number() {
        assert!(version_at_least("2.1.139", "2.1.139"));
        assert!(version_at_least("2.1.140 (Claude Code)", "2.1.139"));
        assert!(version_at_least("2.2.0", "2.1.139"));
        assert!(!version_at_least("2.1.99", "2.1.139"));
        assert!(!version_at_least("2.1.138-beta", "2.1.139"));
    }

    #[test]
    fn paths_compare_like_windows() {
        assert!(same_path(
            Path::new("C:/<Home>/.claude/"),
            Path::new("c:\\<home>\\.CLAUDE")
        ));
        assert!(!same_path(
            Path::new("C:/<Home>/.claude"),
            Path::new("C:/<Home>/.claude-2")
        ));
    }

    #[test]
    fn claude_looks_for_git_bash_in_program_files_on_the_c_drive() {
        let found = claude_program_files();
        let shown: Vec<String> = found.iter().map(|p| p.display().to_string()).collect();
        assert_eq!(
            shown,
            [
                ["C:", "Program Files"].join("\\"),
                ["C:", "Program Files (x86)"].join("\\")
            ]
        );
    }
}
