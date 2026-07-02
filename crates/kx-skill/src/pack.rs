//! The skill PACK loader — the on-disk authoring form of a skill:
//!
//! ```text
//! skills/<name>/
//!   skill.json        # the kortecx.skill/v1 manifest (pack form — no instructions_ref)
//!   instructions.md   # the instructions body (content-addressed at add time)
//!   README.md         # optional, human docs
//! ```
//!
//! Loading is fail-closed: the manifest must validate in pack form, `name` must
//! equal the directory name (the registry-consistency check relies on it), and
//! the instructions body must be non-empty UTF-8 within the size cap. The
//! loader reads exactly these files; the conformance harness — not the loader —
//! asserts a pack contains nothing else (declarative-by-construction).

use std::path::Path;

use crate::manifest::{SkillError, SkillManifest, MAX_SKILL_INSTRUCTIONS_BYTES};

/// A loaded, pack-form-validated skill pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillPack {
    /// The pack-form manifest (`instructions_ref` empty — derived at add time).
    pub manifest: SkillManifest,
    /// The `instructions.md` body (non-empty, ≤ [`MAX_SKILL_INSTRUCTIONS_BYTES`]).
    pub instructions: String,
    /// The `README.md` body, if present.
    pub readme: Option<String>,
}

impl SkillPack {
    /// Load + validate a skill pack from `dir`.
    ///
    /// # Errors
    /// [`SkillError::Pack`] on a missing/oversized/non-UTF-8 file or a
    /// name↔directory mismatch; the [`SkillError`] from
    /// [`SkillManifest::from_json_slice_pack`] on an invalid manifest.
    pub fn load_dir(dir: &Path) -> Result<Self, SkillError> {
        let manifest_bytes = read_file(dir, "skill.json")?;
        let manifest = SkillManifest::from_json_slice_pack(&manifest_bytes)?;

        let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if manifest.name != dir_name {
            return Err(SkillError::Pack(format!(
                "manifest name {:?} must equal the pack directory name {dir_name:?}",
                manifest.name
            )));
        }

        let instructions_bytes = read_file(dir, "instructions.md")?;
        if instructions_bytes.is_empty() {
            return Err(SkillError::Pack(format!(
                "{}/instructions.md is empty — a skill's instructions are its semantic core",
                dir.display()
            )));
        }
        if instructions_bytes.len() > MAX_SKILL_INSTRUCTIONS_BYTES {
            return Err(SkillError::Pack(format!(
                "{}/instructions.md is {} bytes (cap {MAX_SKILL_INSTRUCTIONS_BYTES})",
                dir.display(),
                instructions_bytes.len()
            )));
        }
        let instructions = utf8(dir, "instructions.md", instructions_bytes)?;

        let readme = match std::fs::read(dir.join("README.md")) {
            Ok(bytes) => Some(utf8(dir, "README.md", bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(SkillError::Pack(format!(
                    "{}/README.md: {e}",
                    dir.display()
                )))
            }
        };

        Ok(Self {
            manifest,
            instructions,
            readme,
        })
    }
}

fn read_file(dir: &Path, name: &str) -> Result<Vec<u8>, SkillError> {
    std::fs::read(dir.join(name))
        .map_err(|e| SkillError::Pack(format!("{}/{name}: {e}", dir.display())))
}

fn utf8(dir: &Path, name: &str, bytes: Vec<u8>) -> Result<String, SkillError> {
    String::from_utf8(bytes)
        .map_err(|_| SkillError::Pack(format!("{}/{name} is not valid UTF-8", dir.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_pack(dir: &Path, name: &str, manifest: &str, instructions: &str) {
        let pack = dir.join(name);
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("skill.json"), manifest).unwrap();
        std::fs::write(pack.join("instructions.md"), instructions).unwrap();
    }

    fn manifest(name: &str) -> String {
        format!(
            r#"{{"schema":"kortecx.skill/v1","name":"{name}","version":"1","tools":{{"retrieve":"1"}}}}"#
        )
    }

    #[test]
    fn loads_a_well_formed_pack() {
        let tmp = tempfile::tempdir().unwrap();
        write_pack(tmp.path(), "summarize", &manifest("summarize"), "# Do it\n");
        std::fs::write(tmp.path().join("summarize/README.md"), "docs\n").unwrap();
        let pack = SkillPack::load_dir(&tmp.path().join("summarize")).unwrap();
        assert_eq!(pack.manifest.name, "summarize");
        assert_eq!(pack.instructions, "# Do it\n");
        assert_eq!(pack.readme.as_deref(), Some("docs\n"));
    }

    #[test]
    fn readme_is_optional() {
        let tmp = tempfile::tempdir().unwrap();
        write_pack(tmp.path(), "summarize", &manifest("summarize"), "body");
        let pack = SkillPack::load_dir(&tmp.path().join("summarize")).unwrap();
        assert!(pack.readme.is_none());
    }

    #[test]
    fn name_directory_mismatch_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        write_pack(tmp.path(), "other-dir", &manifest("summarize"), "body");
        let err = SkillPack::load_dir(&tmp.path().join("other-dir")).unwrap_err();
        assert!(err.to_string().contains("directory name"), "{err}");
    }

    #[test]
    fn missing_or_empty_instructions_fail_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let pack = tmp.path().join("summarize");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("skill.json"), manifest("summarize")).unwrap();
        assert!(SkillPack::load_dir(&pack).is_err(), "missing instructions");
        std::fs::write(pack.join("instructions.md"), "").unwrap();
        let err = SkillPack::load_dir(&pack).unwrap_err();
        assert!(err.to_string().contains("empty"), "{err}");
    }

    #[test]
    fn oversized_instructions_fail_closed() {
        let tmp = tempfile::tempdir().unwrap();
        write_pack(
            tmp.path(),
            "summarize",
            &manifest("summarize"),
            &"x".repeat(MAX_SKILL_INSTRUCTIONS_BYTES + 1),
        );
        let err = SkillPack::load_dir(&tmp.path().join("summarize")).unwrap_err();
        assert!(err.to_string().contains("cap"), "{err}");
    }

    #[test]
    fn a_pack_manifest_with_an_instructions_ref_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let stored = format!(
            r#"{{"schema":"kortecx.skill/v1","name":"summarize","instructions_ref":"{}"}}"#,
            "a".repeat(64)
        );
        write_pack(tmp.path(), "summarize", &stored, "body");
        assert!(SkillPack::load_dir(&tmp.path().join("summarize")).is_err());
    }
}
