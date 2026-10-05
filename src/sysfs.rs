use std::fs;
use std::path::Path;

pub fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
}

pub fn read_i64(path: &Path) -> Option<i64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

pub fn read_u64(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{read_i64, read_trimmed, read_u64};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn reads_trimmed_text_and_integers() {
        let dir = TempDir::new().unwrap();
        let text = dir.path().join("text");
        let signed = dir.path().join("signed");
        let unsigned = dir.path().join("unsigned");
        fs::write(&text, "  hello  \n").unwrap();
        fs::write(&signed, "-42\n").unwrap();
        fs::write(&unsigned, "42\n").unwrap();

        assert_eq!(read_trimmed(&text), Some("hello".to_string()));
        assert_eq!(read_i64(&signed), Some(-42));
        assert_eq!(read_u64(&unsigned), Some(42));
    }

    #[test]
    fn invalid_or_missing_values_are_absent() {
        let dir = TempDir::new().unwrap();
        let invalid = dir.path().join("invalid");
        fs::write(&invalid, "not-a-number\n").unwrap();

        assert_eq!(read_i64(&invalid), None);
        assert_eq!(read_u64(&invalid), None);
        assert_eq!(read_trimmed(&dir.path().join("missing")), None);
    }
}
