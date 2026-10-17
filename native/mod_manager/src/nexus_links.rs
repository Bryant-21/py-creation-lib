use crate::{
    database::{Database, error, records},
    encoding, invalid,
    nexus::{GAMES, nexus_game_domain},
};
use pyo3::prelude::*;
use rusqlite::params;

fn game(db: &Database, profile: &str) -> PyResult<String> {
    let rows = records(
        &*db.connection()?,
        "SELECT files FROM profiles WHERE id=?",
        [profile],
    )?;
    let row = rows
        .first()
        .ok_or_else(|| invalid("Select an existing local game profile."))?;
    let files = encoding::parse(row["files"].as_str().unwrap_or("{}"))?;
    let game = nexus_game_domain(files["_modbox_game"].as_str().unwrap_or(""));
    if !GAMES.contains(&game.as_str()) {
        return Err(invalid("This profile has no known Nexus game page."));
    }
    Ok(game)
}

#[pyfunction]
#[pyo3(signature = (db, profile, mod_id=None))]
fn nexus_page(db: &Database, profile: &str, mod_id: Option<&str>) -> PyResult<String> {
    if let Some(mod_id) = mod_id {
        db.connection()?.execute_batch("BEGIN").map_err(error)?;
        let rows = records(
            &*db.connection()?,
            "SELECT m.nexus_game,m.nexus_mod_id FROM mods m JOIN entries e ON e.mod_id=m.id WHERE e.profile_id=? AND m.id=?",
            params![profile, mod_id],
        )?;
        let row = rows
            .first()
            .ok_or_else(|| invalid("Selected mod has no known Nexus mod identity."))?;
        let id = row["nexus_mod_id"]
            .as_u64()
            .filter(|v| *v > 0 && *v < 10_000_000_000)
            .ok_or_else(|| invalid("Selected mod has no known Nexus mod identity."))?;
        let raw = row["nexus_game"].as_str().unwrap_or("");
        let domain = if raw.is_empty() {
            game(db, profile)?
        } else {
            nexus_game_domain(raw)
        };
        if !GAMES.contains(&domain.as_str()) {
            return Err(invalid("Selected mod has no known Nexus game identity."));
        }
        Ok(format!("https://www.nexusmods.com/{domain}/mods/{id}"))
    } else {
        Ok(format!("https://www.nexusmods.com/{}", game(db, profile)?))
    }
}

#[pyfunction]
fn nexus_file_page(game: &str, mod_id: &str, file_id: &str) -> PyResult<String> {
    let mod_id = encoding::parse(mod_id)?;
    crate::nexus::mod_identity(game, &mod_id)?;
    let file_id = crate::nexus_state::identifier(&encoding::parse(file_id)?)?;
    Ok(format!(
        "https://www.nexusmods.com/{game}/mods/{mod_id}?tab=files&file_id={file_id}"
    ))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(nexus_page, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_file_page, m)?)?;
    Ok(())
}
