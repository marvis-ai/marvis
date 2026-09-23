use serde::Serialize;

pub const HUGGING_FACE_PREFIX: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelId {
    Tiny,
    Base,
    Small,
}

impl ModelId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tiny => "tiny",
            Self::Base => "base",
            Self::Small => "small",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCatalogEntry {
    pub id: ModelId,
    pub filename: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    pub sha1: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct VoiceModelInfo {
    pub id: &'static str,
    pub filename: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub bytes: u64,
    pub sha1: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct VoiceModelsCatalog {
    pub models: Vec<VoiceModelInfo>,
}

const CATALOG: [ModelCatalogEntry; 3] = [
    ModelCatalogEntry {
        id: ModelId::Tiny,
        filename: "ggml-tiny.bin",
        label: "Tiny",
        description: "Smallest Whisper model with the fastest transcription.",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
        bytes: 75 * 1024 * 1024,
        sha1: "bd577a113a864445d4c299885e0cb97d4ba92b5f",
    },
    ModelCatalogEntry {
        id: ModelId::Base,
        filename: "ggml-base.bin",
        label: "Base",
        description: "Balanced Whisper model for speed and accuracy.",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
        bytes: 142 * 1024 * 1024,
        sha1: "465707469ff3a37a2b9b8d8f89f2f99de7299dac",
    },
    ModelCatalogEntry {
        id: ModelId::Small,
        filename: "ggml-small.bin",
        label: "Small",
        description: "More accurate Whisper model with a larger footprint.",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        bytes: 466 * 1024 * 1024,
        sha1: "55356645c2b361a969dfd0ef2c5a50d530afd8d5",
    },
];

pub const fn catalog() -> &'static [ModelCatalogEntry] {
    &CATALOG
}

pub fn entry_for_id(value: &str) -> Option<&'static ModelCatalogEntry> {
    let value = value.trim();
    CATALOG
        .iter()
        .find(|entry| entry.id.as_str().eq_ignore_ascii_case(value))
}

pub fn entry_for_filename(value: &str) -> Option<&'static ModelCatalogEntry> {
    let value = value.trim();
    CATALOG.iter().find(|entry| entry.filename == value)
}

pub fn entry_for_value(value: &str) -> Option<&'static ModelCatalogEntry> {
    entry_for_id(value).or_else(|| entry_for_filename(value))
}

impl From<&ModelCatalogEntry> for VoiceModelInfo {
    fn from(entry: &ModelCatalogEntry) -> Self {
        Self {
            id: entry.id.as_str(),
            filename: entry.filename,
            label: entry.label,
            description: entry.description,
            bytes: entry.bytes,
            sha1: entry.sha1,
        }
    }
}

impl From<&'static [ModelCatalogEntry]> for VoiceModelsCatalog {
    fn from(entries: &'static [ModelCatalogEntry]) -> Self {
        Self {
            models: entries.iter().map(VoiceModelInfo::from).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_exactly_the_approved_fixed_catalog() {
        assert_eq!(catalog().len(), 3);
        assert_eq!(
            catalog()
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["tiny", "base", "small"]
        );
        assert_eq!(catalog()[0].filename, "ggml-tiny.bin");
        assert_eq!(catalog()[1].filename, "ggml-base.bin");
        assert_eq!(catalog()[2].filename, "ggml-small.bin");
        assert_eq!(catalog()[0].bytes, 75 * 1024 * 1024);
        assert_eq!(catalog()[1].bytes, 142 * 1024 * 1024);
        assert_eq!(catalog()[2].bytes, 466 * 1024 * 1024);
        assert_eq!(
            catalog()[0].sha1,
            "bd577a113a864445d4c299885e0cb97d4ba92b5f"
        );
        assert_eq!(
            catalog()[1].sha1,
            "465707469ff3a37a2b9b8d8f89f2f99de7299dac"
        );
        assert_eq!(
            catalog()[2].sha1,
            "55356645c2b361a969dfd0ef2c5a50d530afd8d5"
        );
        assert!(catalog().iter().all(|entry| {
            entry.url.starts_with(HUGGING_FACE_PREFIX) && entry.url.starts_with("https://")
        }));
    }

    #[test]
    fn lookup_rejects_arbitrary_ids_urls_and_paths() {
        assert_eq!(entry_for_id(" BASE ").unwrap().id, ModelId::Base);
        assert!(entry_for_id("unknown").is_none());
        for value in [
            "https://example.com/model.bin",
            "../ggml-base.bin",
            "nested/ggml-base.bin",
            r"nested\ggml-base.bin",
            "unsupported.bin",
        ] {
            assert!(entry_for_value(value).is_none(), "accepted {value:?}");
        }
        assert_eq!(
            entry_for_filename(" ggml-small.bin ").unwrap().id,
            ModelId::Small
        );
    }
}
