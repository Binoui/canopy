use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use gtk::prelude::*;

fn main() {
    if let Err(error) = run() {
        eprintln!("Canopy : {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let mut data_dir = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--data-dir") => {
                data_dir = Some(PathBuf::from(
                    args.next().context("--data-dir attend un chemin.")?,
                ))
            }
            Some("--help" | "-h") => {
                println!("Canopy\n\nUsage : canopy [--data-dir CHEMIN]\n\nCtrl+L  Adresse ou recherche\nCtrl+T  Nouvel arbre\nAlt+←   Revenir au parent\nCtrl+R  Recharger la visite\nCtrl+S  Mémoriser la sélection\nCtrl+Q  Quitter\n\nLe profil et la navigation sont conservés dans XDG_DATA_HOME/canopy.");
                return Ok(());
            }
            _ => bail!(
                "Option inconnue : {}. Utilise --help.",
                arg.to_string_lossy()
            ),
        }
    }
    let data_dir = match data_dir {
        Some(path) => path,
        None => std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .context("Définis XDG_DATA_HOME ou utilise --data-dir.")?
            .join("canopy"),
    };
    let new_directory = !data_dir.exists();
    std::fs::create_dir_all(&data_dir)?;
    if new_directory {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    gtk::init().context(
        "Impossible d'ouvrir une fenêtre. Lance Canopy depuis ta session graphique Linux.",
    )?;
    let ui = canopy::ui::Ui::new(&data_dir)?;
    ui.window.connect_delete_event(|_, _| {
        gtk::main_quit();
        glib::Propagation::Proceed
    });
    ui.window.show_all();
    ui.hide_address();
    ui.resume()?;
    gtk::main();
    Ok(())
}
