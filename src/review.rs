use std::collections::BTreeSet;
use std::path::Path;

use regex::Regex;

// A cell's pattern: the differentiating label, full tail after radix, and regex.
#[derive(Debug, Clone, PartialEq)]
pub struct CellPattern {
    pub label: String,
    pub tail: String,
    pub regex_str: String,
}

// Result of analyzing a batch of filenames.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractionResult {
    pub radix: String,
    pub cell_patterns: Vec<CellPattern>,
}

fn longest_common_prefix(strings: &[&str]) -> String {
    if strings.is_empty() {
        return String::new();
    }
    let first = strings[0].as_bytes();
    let mut len = first.len();
    for s in &strings[1..] {
        let b = s.as_bytes();
        len = len.min(b.len());
        for i in 0..len {
            if first[i] != b[i] {
                len = i;
                break;
            }
        }
    }
    strings[0][..len].to_string()
}

// Derive a human-readable label from a tail string.
// Strips leading separators and the file extension.
fn derive_label(tail: &str) -> String {
    let sep_chars: &[char] = &['_', '-', '.'];
    let stripped = tail.trim_start_matches(sep_chars);
    match stripped.rfind('.') {
        Some(pos) => {
            let without_ext = &stripped[..pos];
            if without_ext.is_empty() {
                // tail is like ".jpg" → use extension as label
                stripped[pos + 1..].to_string()
            } else {
                without_ext.to_string()
            }
        }
        None => stripped.to_string(),
    }
}

// Core algorithm: find common prefix across filenames, determine radix boundary,
// then derive per-cell tails, labels, and regexes.
// Each cell stores its full tail (separator + label + extension) so mixed extensions work.
// Returns None if < 2 files or no common structure.
pub fn extract_patterns(filenames: &[&str]) -> Option<ExtractionResult> {
    if filenames.len() < 2 {
        return None;
    }

    let raw_prefix = longest_common_prefix(filenames);
    if raw_prefix.is_empty() {
        return None;
    }

    let sep_chars: &[char] = &['_', '-', '.'];

    let radix = if raw_prefix.ends_with(sep_chars) {
        // Prefix includes trailing separator(s) → trim them to get the radix
        raw_prefix.trim_end_matches(sep_chars)
    } else {
        // Check chars immediately after the prefix in each filename
        let all_non_sep = filenames
            .iter()
            .all(|f| f[raw_prefix.len()..].starts_with(|c: char| !sep_chars.contains(&c)));

        if all_non_sep {
            // We're mid-word (e.g. prefix="frame001_v" from v1/v2), walk back to last separator
            let pos = raw_prefix.rfind(sep_chars)?;
            &raw_prefix[..pos]
        } else {
            // At least one file has a separator/delimiter right after prefix → prefix IS the radix
            &raw_prefix
        }
    };

    if radix.is_empty() {
        return None;
    }

    let mut cell_patterns = Vec::new();
    for &f in filenames {
        let tail = &f[radix.len()..];
        let label = derive_label(tail);
        let regex_str = format!("^(.*){}$", regex::escape(tail));
        cell_patterns.push(CellPattern {
            label,
            tail: tail.to_string(),
            regex_str,
        });
    }

    // Verify all tails are distinct
    let unique_tails: BTreeSet<&str> = cell_patterns.iter().map(|c| c.tail.as_str()).collect();
    if unique_tails.len() != cell_patterns.len() {
        return None;
    }

    Some(ExtractionResult {
        radix: radix.to_string(),
        cell_patterns,
    })
}

// Flat read_dir, match each filename against all cell regexes, collect unique radixes (sorted).
// Only keeps radixes that match at least 2 different cell patterns to filter out false positives
// from broad regexes (e.g. "^(.*)\.jpg$" matching every .jpg file).
pub fn scan_radixes(directory: &Path, cell_patterns: &[CellPattern]) -> Vec<String> {
    use std::collections::HashMap;

    // Compile all regexes upfront, skipping invalid ones
    let compiled: Vec<Option<Regex>> = cell_patterns
        .iter()
        .map(|cp| match Regex::new(&cp.regex_str) {
            Ok(re) => Some(re),
            Err(e) => {
                println!("Invalid regex '{}': {}", cp.regex_str, e);
                None
            }
        })
        .collect();

    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };

    // Track which cell indices each radix matches
    let mut radix_cells: HashMap<String, BTreeSet<usize>> = HashMap::new();

    for entry in entries {
        let Ok(entry) = entry else { continue };
        let Some(name) = entry.file_name().to_str().map(String::from) else {
            continue;
        };
        for (cell_idx, re) in compiled.iter().enumerate() {
            let Some(re) = re else { continue };
            if let Some(caps) = re.captures(&name) {
                if let Some(m) = caps.get(1) {
                    radix_cells.entry(m.as_str().to_string()).or_default().insert(cell_idx);
                }
            }
        }
    }

    // Only keep radixes matching at least 2 cells (or all cells if there's only 1 pattern)
    let min_cells = cell_patterns.len().min(2);
    radix_cells
        .into_iter()
        .filter(|(_, cells)| cells.len() >= min_cells)
        .map(|(radix, _)| radix)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

// For a given radix, find the matching file for each cell by applying the regex against
// directory entries. This works even when the user has manually edited the regex patterns.
// Returns None for cells where no matching file is found.
pub fn resolve_files_for_radix(directory: &Path, radix: &str, cell_patterns: &[CellPattern]) -> Vec<Option<String>> {
    let compiled: Vec<Option<Regex>> = cell_patterns
        .iter()
        .map(|cp| Regex::new(&cp.regex_str).ok())
        .collect();

    let Ok(entries) = std::fs::read_dir(directory) else {
        return vec![None; cell_patterns.len()];
    };

    let mut result: Vec<Option<String>> = vec![None; cell_patterns.len()];

    for entry in entries {
        let Ok(entry) = entry else { continue };
        let Some(name) = entry.file_name().to_str().map(String::from) else {
            continue;
        };
        for (i, re) in compiled.iter().enumerate() {
            if result[i].is_some() {
                continue;
            }
            let Some(re) = re else { continue };
            if let Some(caps) = re.captures(&name) {
                if let Some(m) = caps.get(1) {
                    if m.as_str() == radix {
                        result[i] = Some(directory.join(&name).to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    result
}

#[cfg(test)]
fn extract_radix_from_filename(regex_str: &str, filename: &str) -> Option<String> {
    let re = Regex::new(regex_str).ok()?;
    let caps = re.captures(filename)?;
    caps.get(1).map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    // -- Pattern extraction --

    #[test]
    fn two_variants_same_ext() {
        let result = extract_patterns(&["shot_001_diffuse.jpg", "shot_001_specular.jpg"]).unwrap();
        assert_eq!(result.radix, "shot_001");
        assert_eq!(result.cell_patterns.len(), 2);
        assert_eq!(result.cell_patterns[0].label, "diffuse");
        assert_eq!(result.cell_patterns[0].tail, "_diffuse.jpg");
        assert_eq!(result.cell_patterns[1].label, "specular");
        assert_eq!(result.cell_patterns[1].tail, "_specular.jpg");
    }

    #[test]
    fn mixed_extensions_with_bare_radix() {
        let result = extract_patterns(&["shot_001.jpg", "shot_001_diffuse.tiff", "shot_001_specular.jpeg"]).unwrap();
        assert_eq!(result.radix, "shot_001");
        assert_eq!(result.cell_patterns.len(), 3);
        assert_eq!(result.cell_patterns[0].label, "jpg");
        assert_eq!(result.cell_patterns[0].tail, ".jpg");
        assert_eq!(result.cell_patterns[1].label, "diffuse");
        assert_eq!(result.cell_patterns[1].tail, "_diffuse.tiff");
        assert_eq!(result.cell_patterns[2].label, "specular");
        assert_eq!(result.cell_patterns[2].tail, "_specular.jpeg");
    }

    #[test]
    fn version_suffix() {
        let result = extract_patterns(&["frame001_v1.jpg", "frame001_v2.jpg"]).unwrap();
        assert_eq!(result.radix, "frame001");
        assert_eq!(result.cell_patterns[0].label, "v1");
        assert_eq!(result.cell_patterns[1].label, "v2");
    }

    #[test]
    fn dash_separator() {
        let result = extract_patterns(&["img-001-left.png", "img-001-right.png"]).unwrap();
        assert_eq!(result.radix, "img-001");
        assert_eq!(result.cell_patterns[0].label, "left");
        assert_eq!(result.cell_patterns[1].label, "right");
    }

    #[test]
    fn three_variants_same_ext() {
        let result =
            extract_patterns(&["render_042_beauty.exr", "render_042_depth.exr", "render_042_normal.exr"]).unwrap();
        assert_eq!(result.radix, "render_042");
        assert_eq!(result.cell_patterns.len(), 3);
        assert_eq!(result.cell_patterns[0].label, "beauty");
        assert_eq!(result.cell_patterns[1].label, "depth");
        assert_eq!(result.cell_patterns[2].label, "normal");
    }

    #[test]
    fn single_file_returns_none() {
        assert!(extract_patterns(&["shot_001_diffuse.jpg"]).is_none());
    }

    #[test]
    fn no_common_structure_returns_none() {
        assert!(extract_patterns(&["abc.png", "xyz.jpg"]).is_none());
    }

    // -- Regex matching --

    #[test]
    fn regex_matches_different_radix() {
        let result = extract_patterns(&["shot_001_diffuse.jpg", "shot_001_specular.jpg"]).unwrap();
        let radix = extract_radix_from_filename(&result.cell_patterns[0].regex_str, "shot_042_diffuse.jpg");
        assert_eq!(radix, Some("shot_042".to_string()));
    }

    #[test]
    fn regex_matches_mixed_ext() {
        let result = extract_patterns(&["shot_001.jpg", "shot_001_diffuse.tiff", "shot_001_specular.jpeg"]).unwrap();
        let radix = extract_radix_from_filename(&result.cell_patterns[1].regex_str, "shot_042_diffuse.tiff");
        assert_eq!(radix, Some("shot_042".to_string()));
    }

    #[test]
    fn regex_rejects_unrelated_file() {
        let result = extract_patterns(&["shot_001_diffuse.jpg", "shot_001_specular.jpg"]).unwrap();
        let radix = extract_radix_from_filename(&result.cell_patterns[0].regex_str, "photo_holiday.png");
        assert!(radix.is_none());
    }

    // -- Directory scanning (temp dir with test files) --

    #[test]
    fn scan_finds_radixes_same_ext() {
        let dir = tempfile::tempdir().unwrap();
        for radix in &["shot_001", "shot_002", "shot_003"] {
            for tail in &["_diffuse.jpg", "_specular.jpg"] {
                fs::write(dir.path().join(format!("{}{}", radix, tail)), b"").unwrap();
            }
        }
        fs::write(dir.path().join("unrelated.txt"), b"").unwrap();

        let result = extract_patterns(&["shot_001_diffuse.jpg", "shot_001_specular.jpg"]).unwrap();
        let radixes = scan_radixes(dir.path(), &result.cell_patterns);
        assert_eq!(radixes, vec!["shot_001", "shot_002", "shot_003"]);
    }

    #[test]
    fn scan_finds_radixes_mixed_ext() {
        let dir = tempfile::tempdir().unwrap();
        for radix in &["shot_001", "shot_002", "shot_003"] {
            for tail in &[".jpg", "_diffuse.tiff", "_specular.jpeg"] {
                fs::write(dir.path().join(format!("{}{}", radix, tail)), b"").unwrap();
            }
        }
        fs::write(dir.path().join("unrelated.txt"), b"").unwrap();

        let result = extract_patterns(&["shot_001.jpg", "shot_001_diffuse.tiff", "shot_001_specular.jpeg"]).unwrap();
        let radixes = scan_radixes(dir.path(), &result.cell_patterns);
        assert_eq!(radixes, vec!["shot_001", "shot_002", "shot_003"]);
    }

    #[test]
    fn scan_single_cell_match_filtered_out() {
        let dir = tempfile::tempdir().unwrap();
        // Only one cell matches → radix should be filtered out
        fs::write(dir.path().join("shot_003_diffuse.jpg"), b"").unwrap();

        let result = extract_patterns(&["shot_001_diffuse.jpg", "shot_001_specular.jpg"]).unwrap();
        let radixes = scan_radixes(dir.path(), &result.cell_patterns);
        assert!(radixes.is_empty());
    }

    #[test]
    fn scan_partial_match_two_of_three() {
        let dir = tempfile::tempdir().unwrap();
        // 2 out of 3 cells match → radix should be included
        fs::write(dir.path().join("shot_003.jpg"), b"").unwrap();
        fs::write(dir.path().join("shot_003_diffuse.tiff"), b"").unwrap();
        // shot_003_specular.jpeg is missing

        let result = extract_patterns(&["shot_001.jpg", "shot_001_diffuse.tiff", "shot_001_specular.jpeg"]).unwrap();
        let radixes = scan_radixes(dir.path(), &result.cell_patterns);
        assert_eq!(radixes, vec!["shot_003"]);
    }

    // -- File resolution --

    #[test]
    fn resolve_all_present() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("shot_001_diffuse.jpg"), b"").unwrap();
        fs::write(dir.path().join("shot_001_specular.jpg"), b"").unwrap();

        let result = extract_patterns(&["shot_001_diffuse.jpg", "shot_001_specular.jpg"]).unwrap();
        let files = resolve_files_for_radix(dir.path(), "shot_001", &result.cell_patterns);
        assert!(files[0].is_some());
        assert!(files[1].is_some());
    }

    #[test]
    fn resolve_mixed_ext() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("shot_001.jpg"), b"").unwrap();
        fs::write(dir.path().join("shot_001_diffuse.tiff"), b"").unwrap();
        fs::write(dir.path().join("shot_001_specular.jpeg"), b"").unwrap();

        let result = extract_patterns(&["shot_001.jpg", "shot_001_diffuse.tiff", "shot_001_specular.jpeg"]).unwrap();
        let files = resolve_files_for_radix(dir.path(), "shot_001", &result.cell_patterns);
        assert!(files[0].is_some());
        assert!(files[1].is_some());
        assert!(files[2].is_some());
    }

    #[test]
    fn resolve_missing_cell() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("shot_001_diffuse.jpg"), b"").unwrap();

        let result = extract_patterns(&["shot_001_diffuse.jpg", "shot_001_specular.jpg"]).unwrap();
        let files = resolve_files_for_radix(dir.path(), "shot_001", &result.cell_patterns);
        assert!(files[0].is_some());
        assert!(files[1].is_none());
    }
}
