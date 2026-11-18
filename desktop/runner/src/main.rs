//! pansy: the desktop app, and the `pansy` command.
//!
//! Without arguments it opens the window: the TSX UI compiled into `nsh/`
//! next to this program (on macOS in the bundle's `Resources/nsh`), with
//! `pansy/native` linked in, and the gateway running beside it. With
//! arguments it is the command line (`pansy help`), the same as the
//! headless build's; a `pansy://` link alone opens the window on it (the
//! desktop starts the program with the link the user opened).
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let link = matches!(args.as_slice(), [a] if a.to_ascii_lowercase().starts_with("pansy:"));
    if !args.is_empty() && !link {
        #[cfg(windows)]
        pansy_desktop_native::win::attach_parent_console();
        return pansy_cli::run(args);
    }
    window()
}

/// The `nsh` directory beside the executable (through symlinks), or
/// `PANSY_UI_DIR` for a development build. On macOS it is the bundle's
/// `Contents/Resources/nsh`: `Contents/MacOS` holds only code, or the
/// bundle's signature cannot seal it.
fn package_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PANSY_UI_DIR") {
        return dir.into();
    }
    let resources = if cfg!(target_os = "macos") {
        "../Resources/nsh"
    } else {
        "nsh"
    };
    std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(resources)))
        .unwrap_or_default()
}

/// The app's id and name from `app.json` (written by `nsh build`).
struct AppInfo {
    id: String,
    name: String,
}

impl AppInfo {
    fn read(dir: &std::path::Path) -> AppInfo {
        let info = std::fs::read(dir.join("app.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        let field = |key: &str, default: &str| {
            info.as_ref()
                .and_then(|info| info.get(key)?.as_str().map(str::to_owned))
                .unwrap_or_else(|| default.to_owned())
        };
        AppInfo {
            id: field("id", "dev.pansy.app"),
            name: field("name", "Pansy"),
        }
    }
}

macro_rules! run_window {
    ($platform:ident, $($extra:tt)*) => {{
        use $platform::{AppOptions, AppSource, Appearance, AssetBase, Bundle, Limits};

        let dir = package_dir();
        let info = AppInfo::read(&dir);
        $platform::set_ui_sets(vec![nsh_base::registry(), pansy_desktop_native::registry()]);
        // SAFETY: the bytecode is this package's own, installed beside it.
        let bundle = match unsafe { Bundle::read_trusted(&dir.join("bundle.nshc")) } {
            Ok(bundle) => bundle,
            Err(error) => {
                eprintln!("pansy: cannot read the window's code in {}: {error}", dir.display());
                return ExitCode::FAILURE;
            }
        };
        // A newer build of the window's code, downloaded as a package signed
        // by a key nsh.json's root certifies (ui/Update.tsx), runs instead
        // of the one shipped; one that fails its first run is put away.
        let launch = nsh_engine::updates::launch(bundle, &dir, nsh_engine::support_dir().as_deref());
        // The gateway runs while the window does, the tray keeping both.
        pansy_desktop_native::start_gateway();
        let code = $platform::run_app(
            AppSource::Bundle(launch.bundle),
            AppOptions {
                asset_base: Some(AssetBase::Directory(launch.assets)),
                limits: Limits::standalone(),
                app_id: info.id,
                title: info.name,
                size: (1100, 720),
                appearance: Appearance::System,
                $($extra)*
                ..AppOptions::default()
            },
        );
        pansy_desktop_native::stop_gateway();
        code
    }};
}

#[cfg(target_os = "linux")]
fn window() -> ExitCode {
    // the desktop opens pansy:// links with this program (nsh.json `schemes`)
    let code =
        run_window!(nsh_platform_linux, single_instance: true, schemes: vec!["pansy".to_owned()],);
    if code == nsh_platform_linux::gtk4::glib::ExitCode::SUCCESS {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(windows)]
fn window() -> ExitCode {
    // One window per session: a second start brings the first one forward.
    if !pansy_desktop_native::win::claim_window("Pansy") {
        return ExitCode::SUCCESS;
    }
    run_window!(nsh_platform_windows,)
}

/// LaunchServices keeps one instance of the bundle: opening it again brings
/// this window forward.
#[cfg(target_os = "macos")]
fn window() -> ExitCode {
    run_window!(nsh_platform_macos,)
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
fn window() -> ExitCode {
    let _ = (package_dir(), AppInfo::read);
    eprintln!(
        "pansy: there is no window on this system; `pansy serve` runs the gateway, `pansy help` lists the commands"
    );
    ExitCode::FAILURE
}
