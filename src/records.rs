//! Records sur disque : une ligne « mpm nom » par joueur.

use std::{fs, io, io::Write, path::Path};

use crate::game::{MAX_RECORDS, clean_name};

pub type Records = Vec<(String, u32)>;

pub fn load(path: &Path) -> io::Result<Records> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut records: Records = text
        .lines()
        .filter_map(|l| {
            let (wpm, name) = l.split_once(' ')?;
            let name = clean_name(name);
            (!name.is_empty()).then_some((name, wpm.parse().ok()?))
        })
        .collect();
    records.sort_by_key(|r| std::cmp::Reverse(r.1));
    records.truncate(MAX_RECORDS);
    Ok(records)
}

/// Écrit à côté puis renomme : jamais de fichier à moitié écrit.
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut f = fs::File::create(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    fs::rename(&tmp, path)
}

pub fn save(path: &Path, records: &Records) -> io::Result<()> {
    let text: String = records.iter().map(|(n, w)| format!("{w} {n}\n")).collect();
    write_atomic(path, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_bad_lines() {
        let path = std::env::temp_dir().join(format!("ssh-typing-{}.txt", std::process::id()));
        assert!(load(&path).unwrap().is_empty());
        let records = vec![("ada".to_string(), 120), ("bob".to_string(), 80)];
        save(&path, &records).unwrap();
        assert_eq!(load(&path).unwrap(), records);
        fs::write(&path, "n\u{1b}importe quoi\n90 eve\n\n100 \n").unwrap();
        assert_eq!(load(&path).unwrap(), vec![("eve".to_string(), 90)]);
        fs::remove_file(&path).unwrap();
    }
}
