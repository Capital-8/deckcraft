//! Optional craft-fonts build input (https://github.com/storytold/craft-fonts, recipe from its
//! `docs/integration.md`). With `CRAFT_FONTS_DIR=<checkout>` the fonts in its
//! `fonts/latin-manifest.txt` (presentation fonts) and `fonts/manifest.txt` are embedded as
//! `CRAFT_FONTS`, Latin first. Native builds additionally read
//! `fonts/simplified-chinese-manifest.txt`; it is optional for ordinary local builds but a required
//! native input (`CRAFT_FONTS_REQUIRED`) fails without it. Unset, `CRAFT_FONTS` is empty. Web
//! (wasm32) builds embed only Inter
//! Regular/Bold and BIZ UDPGothic Regular, to stay within the web size budget. It only reads the
//! local checkout: no network.
use std::fmt::Write as _;
use std::path::PathBuf;

/// Families DeckCraft never embeds (AGENTS.md §1: no Adobe-authored visual design, and no Noto
/// CJK rebrand of Source Han on any platform).
const EXCLUDED_FAMILIES: &[&str] = &["Source Sans 3", "Source Serif 4", "Source Han Sans", "Source Han Serif"];

fn is_excluded_family(family: &str) -> bool {
    EXCLUDED_FAMILIES.contains(&family)
        || family.starts_with("Source Han Sans")
        || family.starts_with("Source Han Serif")
        || family.starts_with("Noto Sans CJK")
        || family.starts_with("Noto Serif CJK")
        || family.starts_with("Noto Sans Mono CJK")
}

fn main() {
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_REQUIRED");
    let mut src = String::from("pub static CRAFT_FONTS: &[CraftFont] = &[\n");
    let dir = std::env::var_os("CRAFT_FONTS_DIR").map(PathBuf::from);
    let required = std::env::var_os("CRAFT_FONTS_REQUIRED").is_some();
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    match input_fonts(dir.as_deref(), wasm, required) {
        Ok(entries) => src.push_str(&entries),
        Err(e) if required => println!("cargo::error={e}"),
        Err(e) => println!("cargo::warning=building without craft-fonts: {e}"),
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("craft_fonts.rs");
    if let Err(e) = std::fs::write(&out, src) {
        println!("cargo::error=writing {}: {e}", out.display());
    }
}

/// One `CraftFont { .. }` initialiser per manifest line.
fn input_fonts(dir: Option<&std::path::Path>, wasm: bool, required: bool) -> Result<String, String> {
    match dir {
        Some(dir) => craft_fonts(dir, wasm, required),
        None if required => Err("CRAFT_FONTS_REQUIRED is set but CRAFT_FONTS_DIR is missing".into()),
        None => Ok(String::new()),
    }
}

fn craft_fonts(dir: &std::path::Path, wasm: bool, required: bool) -> Result<String, String> {
    let mut out = String::new();
    // The Latin presentation fonts are optional within craft-fonts (older checkouts lack them).
    let latin = dir.join("fonts/latin-manifest.txt");
    println!("cargo::rerun-if-changed={}", latin.display());
    if latin.exists() {
        out.push_str(&manifest(dir, &latin, wasm)?);
    }
    out.push_str(&manifest(dir, &dir.join("fonts/manifest.txt"), wasm)?);
    // Native consumers opt in; older craft-fonts checkouts need not carry this manifest.
    // An invalid optional addition must never discard the main Japanese/symbol/RTL faces.
    if !wasm {
        let chinese = dir.join("fonts/simplified-chinese-manifest.txt");
        println!("cargo::rerun-if-changed={}", chinese.display());
        let exists = match chinese.try_exists() {
            Ok(exists) => exists,
            Err(e) if required => return Err(format!("{}: {e}", chinese.display())),
            Err(e) => {
                println!("cargo::warning=skipping optional Chinese fonts: {}: {e}", chinese.display());
                false
            }
        };
        if exists {
            match manifest(dir, &chinese, false) {
                Ok(entries) => out.push_str(&entries),
                Err(e) if required => return Err(e),
                Err(e) => println!("cargo::warning=skipping optional Chinese fonts: {e}"),
            }
        } else if required {
            // A required native font input that silently lacks the reviewed Chinese manifest
            // would make the Chinese UI/canvas test pass only by skipping.
            return Err(format!("{}: CRAFT_FONTS_REQUIRED is set but the simplified-Chinese font manifest is missing", chinese.display()));
        }
    }
    Ok(out)
}

fn manifest(dir: &std::path::Path, manifest: &std::path::Path, wasm: bool) -> Result<String, String> {
    println!("cargo::rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut out = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split(" | ").map(str::trim).collect();
        let [family, style, file, scripts, ..] = f.as_slice() else {
            return Err(format!("malformed manifest line: {line}"));
        };
        // DeckCraft's asset policy: no Adobe-authored typefaces, even openly licensed ones.
        if is_excluded_family(family) {
            continue;
        }
        if wasm && !matches!((*family, *style), ("BIZ UDPGothic", "Regular") | ("Inter", "Regular" | "Bold" | "Variable")) {
            continue;
        }
        let path = dir.join(file).canonicalize().map_err(|e| format!("{file}: {e}"))?;
        println!("cargo::rerun-if-changed={}", path.display());
        let scripts: Vec<String> = scripts.split(',').map(|s| format!("{:?}", s.trim())).collect();
        let _ = writeln!(
            out,
            "    CraftFont {{ family: {family:?}, style: {style:?}, scripts: &[{}], bytes: include_bytes!({:?}) }},",
            scripts.join(", "),
            path.display().to_string(),
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct InputFixture(PathBuf);

    impl InputFixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("deck-font-input-{}-{}", std::process::id(), NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&path).unwrap();
            std::fs::create_dir(path.join("fonts")).unwrap();
            std::fs::write(path.join("fonts/fixture.ttf"), b"build-input path fixture").unwrap();
            std::fs::write(
                path.join("fonts/manifest.txt"),
                "BIZ UDPGothic | Regular | fonts/fixture.ttf | Jpan,Latn | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n",
            )
            .unwrap();
            Self(path)
        }

        fn optional(&self, text: &str) {
            std::fs::write(self.0.join("fonts/simplified-chinese-manifest.txt"), text).unwrap();
        }
    }

    impl Drop for InputFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn required_input_cannot_silently_disappear() {
        assert!(input_fonts(None, false, true).unwrap_err().contains("CRAFT_FONTS_DIR"));
        assert_eq!(input_fonts(None, false, false).unwrap(), "");
    }

    #[test]
    fn broken_optional_manifest_preserves_the_main_fonts() {
        let fixture = InputFixture::new();
        let main = input_fonts(Some(&fixture.0), false, false).unwrap();
        fixture.optional("malformed row");
        assert_eq!(input_fonts(Some(&fixture.0), false, false).unwrap(), main);
        assert!(input_fonts(Some(&fixture.0), false, true).is_err());
        fixture.optional("LXGW WenKai Lite | Regular | missing.ttf | Hani,Hans | OFL-1.1 | OFL.txt | fixture | https://example.invalid");
        assert_eq!(input_fonts(Some(&fixture.0), false, false).unwrap(), main);
        assert!(input_fonts(Some(&fixture.0), false, true).is_err());
    }

    #[test]
    fn native_opt_in_does_not_change_wasm_embedded_bytes() {
        let fixture = InputFixture::new();
        let before = input_fonts(Some(&fixture.0), true, true).unwrap();
        fixture.optional("LXGW WenKai Lite | Regular | fonts/fixture.ttf | Hani,Hans | OFL-1.1 | OFL.txt | fixture | https://example.invalid");
        let native = input_fonts(Some(&fixture.0), false, true).unwrap();
        assert!(native.contains("LXGW WenKai Lite"));
        assert_eq!(input_fonts(Some(&fixture.0), true, true).unwrap(), before);
        fixture.optional("malformed row");
        assert_eq!(input_fonts(Some(&fixture.0), true, true).unwrap(), before);
    }

    #[test]
    fn required_native_input_requires_the_chinese_manifest() {
        let fixture = InputFixture::new();
        assert!(input_fonts(Some(&fixture.0), false, true).unwrap_err().contains("simplified-Chinese"));
        // Optional native builds and wasm keep working without it.
        assert!(input_fonts(Some(&fixture.0), false, false).is_ok());
        assert!(input_fonts(Some(&fixture.0), true, true).is_ok());
    }

    #[test]
    fn all_noto_cjk_families_are_excluded_from_every_platform() {
        for family in [
            "Noto Sans CJK SC",
            "Noto Sans CJK TC",
            "Noto Sans CJK JP",
            "Noto Sans CJK KR",
            "Noto Sans CJK HK",
            "Noto Serif CJK SC",
            "Noto Serif CJK JP",
            "Noto Sans Mono CJK SC",
            "Source Han Sans SC",
            "Source Han Serif JP",
        ] {
            assert!(is_excluded_family(family), "{family} would be embedded");
        }
        for family in ["LXGW WenKai Lite", "Noto Sans Hebrew", "Inter", "Noto Sans Symbols 2"] {
            assert!(!is_excluded_family(family), "{family} is an allowed input");
        }
        let fixture = InputFixture::new();
        let mut rows = String::new();
        for family in ["Noto Sans CJK SC", "Noto Serif CJK JP", "Noto Sans Mono CJK KR"] {
            rows.push_str(&format!("{family} | Regular | fonts/fixture.ttf | Hani | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n"));
        }
        fixture.optional(&rows);
        let native = input_fonts(Some(&fixture.0), false, true).unwrap();
        assert!(!native.contains("Noto Sans CJK") && !native.contains("Noto Serif CJK") && !native.contains("Noto Sans Mono CJK"));
        assert!(native.contains("BIZ UDPGothic"), "the allowed main manifest is retained");
    }
}
