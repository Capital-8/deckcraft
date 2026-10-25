//! Optional craft-fonts build input (https://github.com/storytold/craft-fonts, recipe from its
//! `docs/integration.md`). With `CRAFT_FONTS_DIR=<checkout>` the fonts in its
//! `fonts/latin-manifest.txt` (presentation fonts) and `fonts/manifest.txt` are embedded as
//! `CRAFT_FONTS`, Latin first. Native builds additionally read
//! `fonts/simplified-chinese-manifest.txt`; it is optional for ordinary local builds but a required
//! native input (`CRAFT_FONTS_REQUIRED`) fails without it. Unset, `CRAFT_FONTS` is empty. Web
//! (wasm32) builds embed only Inter Regular/Bold (or a supported Variable face) and BIZ UDPGothic
//! Regular, to stay within the web size budget. Required builds also verify those faces survive
//! exclusion (and, on wasm, the web filter), so a pin without the Latin manifest fails instead of
//! silently dropping Inter. It only reads the local checkout: no network.
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

/// The generated initialisers plus the faces that were actually embedded after exclusion and web
/// filtering. Validation must use the latter, not the raw manifest text.
#[derive(Default)]
struct FontInput {
    source: String,
    faces: Vec<(String, String)>,
}

impl FontInput {
    fn append(&mut self, other: Self) {
        self.source.push_str(&other.source);
        self.faces.extend(other.faces);
    }
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
    load_fonts(dir, wasm, required).map(|input| input.source)
}

fn load_fonts(dir: Option<&std::path::Path>, wasm: bool, required: bool) -> Result<FontInput, String> {
    match dir {
        Some(dir) => craft_fonts(dir, wasm, required),
        None if required => Err("CRAFT_FONTS_REQUIRED is set but CRAFT_FONTS_DIR is missing".into()),
        None => Ok(FontInput::default()),
    }
}

fn craft_fonts(dir: &std::path::Path, wasm: bool, required: bool) -> Result<FontInput, String> {
    let mut input = FontInput::default();
    // The Latin presentation fonts are optional within craft-fonts (older checkouts lack them).
    // A malformed or incomplete optional manifest is skipped as a unit: the main Japanese/symbol/RTL
    // faces and the native Chinese addition stay usable, and required builds still fail validation.
    let latin = dir.join("fonts/latin-manifest.txt");
    println!("cargo::rerun-if-changed={}", latin.display());
    let latin_exists = match latin.try_exists() {
        Ok(exists) => exists,
        Err(e) if required => return Err(format!("{}: {e}", latin.display())),
        Err(e) => {
            println!("cargo::warning=skipping optional Latin fonts: {}: {e}", latin.display());
            false
        }
    };
    if latin_exists {
        match manifest(dir, &latin, wasm) {
            Ok(entries) => input.append(entries),
            Err(e) if required => return Err(e),
            Err(e) => println!("cargo::warning=skipping optional Latin fonts: {e}"),
        }
    }
    input.append(manifest(dir, &dir.join("fonts/manifest.txt"), wasm)?);
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
                Ok(entries) => input.append(entries),
                Err(e) if required => return Err(e),
                Err(e) => println!("cargo::warning=skipping optional Chinese fonts: {e}"),
            }
        } else if required {
            // A required native font input that silently lacks the reviewed Chinese manifest
            // would make the Chinese UI/canvas test pass only by skipping.
            return Err(format!("{}: CRAFT_FONTS_REQUIRED is set but the simplified-Chinese font manifest is missing", chinese.display()));
        }
    }
    if required {
        validate_required_faces(&input.faces, wasm)?;
    }
    Ok(input)
}

fn manifest(dir: &std::path::Path, manifest: &std::path::Path, wasm: bool) -> Result<FontInput, String> {
    println!("cargo::rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut input = FontInput::default();
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
        input.faces.push(((*family).to_owned(), (*style).to_owned()));
        let _ = writeln!(
            input.source,
            "    CraftFont {{ family: {family:?}, style: {style:?}, scripts: &[{}], bytes: include_bytes!({:?}) }},",
            scripts.join(", "),
            path.display().to_string(),
        );
    }
    Ok(input)
}

/// Required builds must actually carry the faces the platform promises: Inter Regular and Bold
/// (or one Variable face) and BIZ UDPGothic Regular everywhere, plus LXGW WenKai Lite Regular for
/// native builds. This runs after exclusion and, on wasm, the web filter, so an absent manifest,
/// empty/all-excluded rows, a wrong-family row or a partial catalog cannot satisfy the gate.
fn validate_required_faces(faces: &[(String, String)], wasm: bool) -> Result<(), String> {
    let has = |family: &str, style: &str| faces.iter().any(|(f, s)| f == family && s == style);
    let inter = has("Inter", "Variable") || (has("Inter", "Regular") && has("Inter", "Bold"));
    let biz = has("BIZ UDPGothic", "Regular");
    let chinese = wasm || has("LXGW WenKai Lite", "Regular");
    let mut missing = Vec::new();
    if !inter {
        missing.push("Inter Regular/Bold (or Inter Variable)");
    }
    if !biz {
        missing.push("BIZ UDPGothic Regular");
    }
    if !chinese {
        missing.push("LXGW WenKai Lite Regular");
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("CRAFT_FONTS_REQUIRED is set but the embedded font set after filtering lacks {}", missing.join(" and ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    const CHINESE_ROW: &str = "LXGW WenKai Lite | Regular | fonts/fixture.ttf | Hani,Hans | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n";

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

        fn main(&self, text: &str) {
            std::fs::write(self.0.join("fonts/manifest.txt"), text).unwrap();
        }

        fn latin(&self, text: &str) {
            std::fs::write(self.0.join("fonts/latin-manifest.txt"), text).unwrap();
        }

        fn no_latin(&self) {
            let _ = std::fs::remove_file(self.0.join("fonts/latin-manifest.txt"));
        }

        fn valid_latin(&self) {
            self.latin(
                "Inter | Regular | fonts/fixture.ttf | Latn | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n\
                 Inter | Bold | fonts/fixture.ttf | Latn | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n",
            );
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
        fixture.valid_latin();
        let before = input_fonts(Some(&fixture.0), true, true).unwrap();
        fixture.optional(CHINESE_ROW);
        let native = input_fonts(Some(&fixture.0), false, true).unwrap();
        assert!(native.contains("LXGW WenKai Lite"));
        assert_eq!(input_fonts(Some(&fixture.0), true, true).unwrap(), before);
        fixture.optional("malformed row");
        assert_eq!(input_fonts(Some(&fixture.0), true, true).unwrap(), before);
    }

    #[test]
    fn required_native_input_requires_the_chinese_manifest() {
        let fixture = InputFixture::new();
        fixture.valid_latin();
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
        fixture.valid_latin();
        let mut rows = String::from(CHINESE_ROW);
        for family in ["Noto Sans CJK SC", "Noto Serif CJK JP", "Noto Sans Mono CJK KR"] {
            rows.push_str(&format!("{family} | Regular | fonts/fixture.ttf | Hani | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n"));
        }
        fixture.optional(&rows);
        let native = input_fonts(Some(&fixture.0), false, true).unwrap();
        assert!(!native.contains("Noto Sans CJK") && !native.contains("Noto Serif CJK") && !native.contains("Noto Sans Mono CJK"));
        assert!(native.contains("BIZ UDPGothic"), "the allowed main manifest is retained");
        assert!(native.contains("LXGW WenKai Lite"), "the valid Chinese row is retained");
    }

    #[test]
    fn required_native_chinese_manifest_must_emit_the_reviewed_face() {
        let fixture = InputFixture::new();
        fixture.valid_latin();
        let excluded = "Noto Sans CJK SC | Regular | fonts/fixture.ttf | Hani | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n\
                        Noto Serif CJK JP | Regular | fonts/fixture.ttf | Hani | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n";
        let wrong_family = "LXGW WenKai | Regular | fonts/fixture.ttf | Hani,Hans | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n";
        for rows in ["", excluded, wrong_family] {
            fixture.optional(rows);
            let e = input_fonts(Some(&fixture.0), false, true).unwrap_err();
            assert!(e.contains("LXGW WenKai Lite Regular"), "rows={rows:?}: {e}");
            assert!(input_fonts(Some(&fixture.0), false, false).is_ok(), "optional builds still preserve the valid core");
        }
        fixture.optional(CHINESE_ROW);
        let native = input_fonts(Some(&fixture.0), false, true).unwrap();
        assert!(native.contains("CraftFont { family: \"LXGW WenKai Lite\", style: \"Regular\""), "{native}");
        assert!(native.contains("BIZ UDPGothic") && native.contains("Inter"), "the rest of the required catalog is retained");
    }

    #[test]
    fn optional_broken_latin_manifest_is_skipped_without_losing_core_or_chinese() {
        let fixture = InputFixture::new();
        fixture.optional(CHINESE_ROW);
        let missing_asset = "Inter | Regular | fonts/missing.ttf | Latn | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n";
        for latin in [None, Some("malformed row"), Some(missing_asset)] {
            match latin {
                Some(text) => fixture.latin(text),
                None => fixture.no_latin(),
            }
            for wasm in [false, true] {
                let catalog = input_fonts(Some(&fixture.0), wasm, false).unwrap();
                assert!(catalog.contains("BIZ UDPGothic"), "wasm={wasm}: the main manifest is retained");
                assert!(!catalog.contains("Inter"), "wasm={wasm}: the broken Latin manifest is skipped as a unit");
                if !wasm {
                    assert!(catalog.contains("LXGW WenKai Lite"), "the valid Chinese addition is retained");
                }
            }
            assert!(input_fonts(Some(&fixture.0), false, true).is_err(), "required builds must not skip the missing required faces");
        }
    }

    #[test]
    fn full_prior_latin_catalog_keeps_permitted_native_families() {
        let fixture = InputFixture::new();
        fixture.latin(include_str!("tests/fixtures/latin-families.txt"));
        fixture.optional(CHINESE_ROW);
        let native = input_fonts(Some(&fixture.0), false, true).unwrap();
        assert_eq!(native.matches("CraftFont {").count(), 45, "43 permitted Latin faces plus BIZ and LXGW");
        for family in [
            "Carlito",
            "Caladea",
            "Liberation Serif",
            "Montserrat",
            "Lato",
            "Open Sans",
            "Roboto",
            "Merriweather",
            "Playfair Display",
            "Poppins",
            "Nunito Sans",
        ] {
            assert!(native.contains(family), "missing prior presentation family {family}");
        }
        let web = input_fonts(Some(&fixture.0), true, true).unwrap();
        assert_eq!(web.matches("CraftFont {").count(), 3, "Inter Regular/Bold and BIZ only");
        for catalog in [&native, &web] {
            assert!(!catalog.contains("Source Sans 3"));
            assert!(!catalog.contains("Source Serif 4"));
        }
    }

    #[test]
    fn required_builds_need_the_latin_catalog_for_inter() {
        let fixture = InputFixture::new();
        fixture.optional(CHINESE_ROW);
        for wasm in [true, false] {
            let e = input_fonts(Some(&fixture.0), wasm, true).unwrap_err();
            assert!(e.contains("Inter Regular/Bold"), "wasm={wasm}: {e}");
            assert!(input_fonts(Some(&fixture.0), wasm, false).is_ok(), "optional builds still work");
        }
    }

    #[test]
    fn required_builds_reject_partial_or_unrelated_faces() {
        let fixture = InputFixture::new();
        fixture.optional(CHINESE_ROW);
        for latin in [
            "Inter | Regular | fonts/fixture.ttf | Latn | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n",
            "Inter | Bold | fonts/fixture.ttf | Latn | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n",
            "Inter | Italic | fonts/fixture.ttf | Latn | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n\
             Inter | SemiBold | fonts/fixture.ttf | Latn | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n",
            "Carlito | Regular | fonts/fixture.ttf | Latn | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n",
            "Source Sans 3 | Regular | fonts/fixture.ttf | Latn | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n\
             Source Sans 3 | Bold | fonts/fixture.ttf | Latn | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n",
        ] {
            fixture.latin(latin);
            for wasm in [true, false] {
                let e = input_fonts(Some(&fixture.0), wasm, true).unwrap_err();
                assert!(e.contains("Inter Regular/Bold"), "wasm={wasm}: {e}");
                assert!(input_fonts(Some(&fixture.0), wasm, false).is_ok(), "optional builds still work");
            }
        }
    }

    #[test]
    fn required_builds_need_biz_udpgothic_regular() {
        let fixture = InputFixture::new();
        fixture.valid_latin();
        fixture.main(
            "Inter | Regular | fonts/fixture.ttf | Latn | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n\
             Inter | Bold | fonts/fixture.ttf | Latn | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n",
        );
        fixture.optional(CHINESE_ROW);
        for wasm in [true, false] {
            let e = input_fonts(Some(&fixture.0), wasm, true).unwrap_err();
            assert!(e.contains("BIZ UDPGothic Regular"), "wasm={wasm}: {e}");
            assert!(input_fonts(Some(&fixture.0), wasm, false).is_ok(), "optional builds still work");
        }
    }

    #[test]
    fn required_builds_accept_static_or_variable_inter_with_biz() {
        let fixture = InputFixture::new();
        fixture.valid_latin();
        fixture.optional(CHINESE_ROW);
        for wasm in [true, false] {
            let static_catalog = input_fonts(Some(&fixture.0), wasm, true).unwrap();
            assert!(static_catalog.contains("Inter") && static_catalog.contains("BIZ UDPGothic"));
        }
        fixture.latin("Inter | Variable | fonts/fixture.ttf | Latn | OFL-1.1 | OFL.txt | fixture | https://example.invalid\n");
        for wasm in [true, false] {
            let variable_catalog = input_fonts(Some(&fixture.0), wasm, true).unwrap();
            assert!(variable_catalog.contains("Inter") && variable_catalog.contains("BIZ UDPGothic"));
        }
    }
}
