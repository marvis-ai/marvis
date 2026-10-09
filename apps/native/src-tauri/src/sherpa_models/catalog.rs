use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SherpaModelId {
    SenseVoice,
    SpeakerEmbedding,
    PunctEn,
    PunctZh,
}
impl SherpaModelId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SenseVoice => "sense-voice",
            Self::SpeakerEmbedding => "speaker-id",
            Self::PunctEn => "punct-en",
            Self::PunctZh => "punct-zh",
        }
    }
}

/// What a catalog entry is for. STT entries can be selected as the
/// transcription model; `SpeakerEmbedding` entries only feed speaker
/// diarization and `Punctuation` entries only post-process sherpa
/// transcripts — neither may appear as a selectable STT model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SherpaModelKind {
    Stt,
    SpeakerEmbedding,
    Punctuation,
}
impl SherpaModelKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stt => "stt",
            Self::SpeakerEmbedding => "speaker-embedding",
            Self::Punctuation => "punctuation",
        }
    }
}

/// One pinned file of a catalog entry — the sherpa "model" is a set of files
/// (recognizer, tokens, VAD) installed together under `root/<dirname>/`.
#[derive(Debug, Clone, Copy)]
pub struct SherpaFileSpec {
    pub filename: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct SherpaCatalogEntry {
    pub id: SherpaModelId,
    pub dirname: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub files: &'static [SherpaFileSpec],
    pub kind: SherpaModelKind,
}

pub(super) const SENSE_VOICE_FILES: &[SherpaFileSpec] = &[
    SherpaFileSpec {
        filename: "model.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09/resolve/main/model.int8.onnx",
        bytes: 237_115_547,
        sha256: "12ca1a2ae7ecf3e0019ef2822307ee0b5cadc9196569e379b4c4026f8205276d",
    },
    SherpaFileSpec {
        filename: "tokens.txt",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09/resolve/main/tokens.txt",
        bytes: 315_894,
        sha256: "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc",
    },
    SherpaFileSpec {
        filename: "silero_vad.onnx",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx",
        bytes: 643_854,
        sha256: "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6",
    },
];

pub(super) const SPEAKER_EMBEDDING_FILES: &[SherpaFileSpec] = &[SherpaFileSpec {
    filename: "3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx",
    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx",
    bytes: 28_281_164,
    sha256: "aa3cfc16963a10586a9393f5035d6d6b57e98d358b347f80c2a30bf4f00ceba2",
}];

// sherpa-onnx-online-punct-en-2024-08-06 — CNN-BiLSTM punctuation + casing
// head from the `punctuation-models` GitHub release. Hugging Face hosts only
// community mirrors of the loose files; both pinned digests were verified
// byte-identical to the official release tarball.
pub(super) const PUNCT_EN_FILES: &[SherpaFileSpec] = &[
    SherpaFileSpec {
        filename: "model.int8.onnx",
        url: "https://huggingface.co/lorneluo/sherpa-onnx-online-punct-en-2024-08-06/resolve/main/model.int8.onnx",
        bytes: 7_490_500,
        sha256: "9d611f445fe4a46186080fe161be6059d87d72eb88d3a8cb00c1a06e83a6067e",
    },
    SherpaFileSpec {
        filename: "bpe.vocab",
        url: "https://huggingface.co/lorneluo/sherpa-onnx-online-punct-en-2024-08-06/resolve/main/bpe.vocab",
        bytes: 149_430,
        sha256: "e118b7ad88c54db562517df49e1cffd4836d166c34fb190fd311d7f34eb238f5",
    },
];

// sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8 —
// CT-Transformer punctuation for zh + en text. SenseVoice's `use_itn` is
// a no-op in the pinned 2025-09-09 export (upstream k2-fsa/sherpa-onnx
// #2742), so zh segments reach the transcript with no punctuation at all
// without this. The pinned digest was verified byte-identical to the
// official `punctuation-models` release tarball.
pub(super) const PUNCT_ZH_FILES: &[SherpaFileSpec] = &[SherpaFileSpec {
    filename: "model.int8.onnx",
    url: "https://huggingface.co/lorneluo/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8/resolve/main/model.int8.onnx",
    bytes: 75_519_198,
    sha256: "65a3fb9f5ad7bfb96bf69e0dc4481df97f6ee60513c1d94ce981ba6effd524b1",
}];

pub(super) const CATALOG: [SherpaCatalogEntry; 4] = [
    SherpaCatalogEntry {
        id: SherpaModelId::SenseVoice,
        dirname: "sense-voice",
        label: "SenseVoice",
        description: "Transcribes Chinese, English, Japanese, Korean, and Cantonese.",
        files: SENSE_VOICE_FILES,
        kind: SherpaModelKind::Stt,
    },
    SherpaCatalogEntry {
        id: SherpaModelId::SpeakerEmbedding,
        dirname: "speaker-id",
        label: "Speaker ID",
        description: "Recognizes different voices so each speaker gets a label.",
        files: SPEAKER_EMBEDDING_FILES,
        kind: SherpaModelKind::SpeakerEmbedding,
    },
    SherpaCatalogEntry {
        id: SherpaModelId::PunctEn,
        dirname: "punct-en",
        label: "Punctuation (English)",
        description: "Restores English punctuation and capitalization.",
        files: PUNCT_EN_FILES,
        kind: SherpaModelKind::Punctuation,
    },
    SherpaCatalogEntry {
        id: SherpaModelId::PunctZh,
        dirname: "punct-zh",
        label: "Punctuation (中文)",
        description: "Restores Chinese punctuation in Chinese and mixed text.",
        files: PUNCT_ZH_FILES,
        kind: SherpaModelKind::Punctuation,
    },
];

pub const fn catalog() -> &'static [SherpaCatalogEntry] {
    &CATALOG
}
/// Match a catalog id/dirname loosely — case-insensitive and indifferent to
/// `-`/`_`/space separators — so the user-facing "SenseVoice" resolves to
/// "sense-voice". Every other character must match exactly, so URLs, paths,
/// and unrelated names still fail.
pub(super) fn name_eq(name: &str, value: &str) -> bool {
    let normalize = |s: &str| -> String {
        s.trim()
            .chars()
            .filter(|c| !matches!(c, '-' | '_' | ' '))
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    normalize(name) == normalize(value)
}
pub fn entry_for_id(value: &str) -> Option<&'static SherpaCatalogEntry> {
    CATALOG.iter().find(|e| name_eq(e.id.as_str(), value))
}
pub(super) fn entry_for_dirname(value: &str) -> Option<&'static SherpaCatalogEntry> {
    CATALOG.iter().find(|e| name_eq(e.dirname, value))
}
/// Resolve a user-facing value to a catalog entry by id or dirname — never a
/// path or URL, so downloads and deletes can only touch the pinned set.
pub fn entry_for_value(value: &str) -> Option<&'static SherpaCatalogEntry> {
    entry_for_id(value).or_else(|| entry_for_dirname(value))
}
/// STT-selectable subset of `entry_for_value` — non-transcription entries
/// (speaker embeddings) resolve to `None` here so they can never be stored
/// as `models.stt_model`.
pub fn stt_entry_for_value(value: &str) -> Option<&'static SherpaCatalogEntry> {
    entry_for_value(value).filter(|entry| entry.kind == SherpaModelKind::Stt)
}
/// `root/<dirname>/` — the directory that owns every file of an entry.
pub fn entry_dir(root: &Path, entry: &SherpaCatalogEntry) -> PathBuf {
    root.join(entry.dirname)
}
/// Path to the speaker-embedding ONNX when its catalog entry is installed.
pub fn speaker_embedding_model_path(root: &Path) -> Option<PathBuf> {
    let entry = entry_for_id(SherpaModelId::SpeakerEmbedding.as_str())?;
    let path = entry_dir(root, entry).join(entry.files[0].filename);
    path.is_file().then_some(path)
}
/// Paths `(cnn_bilstm, bpe_vocab)` of the English punctuation model when its
/// catalog entry is installed — `None` keeps sherpa transcripts unmodified.
pub(super) fn punctuation_en_paths(root: &Path) -> Option<(PathBuf, PathBuf)> {
    let entry = entry_for_id(SherpaModelId::PunctEn.as_str())?;
    let dir = entry_dir(root, entry);
    entry_installed_at(root, entry).then(|| (dir.join("model.int8.onnx"), dir.join("bpe.vocab")))
}
/// Path of the zh-en CT-Transformer punctuator when its catalog entry is
/// installed — `None` leaves zh segments as the recognizer emits them.
pub(super) fn punctuation_zh_path(root: &Path) -> Option<PathBuf> {
    let entry = entry_for_id(SherpaModelId::PunctZh.as_str())?;
    let path = entry_dir(root, entry).join(entry.files[0].filename);
    entry_installed_at(root, entry).then_some(path)
}
/// The optional punctuation add-ons' installed paths — `en` is the
/// CNN-BiLSTM `(model, vocab)` pair, `zh` the CT-Transformer model. A
/// `None` side passes that script's text through unmodified.
pub struct PunctuationPaths {
    pub en: Option<(PathBuf, PathBuf)>,
    pub zh: Option<PathBuf>,
}
/// Installed punctuation paths for the speech worker — one lookup keeps
/// the en/zh pair symmetric.
pub fn punctuation_paths(root: &Path) -> PunctuationPaths {
    PunctuationPaths {
        en: punctuation_en_paths(root),
        zh: punctuation_zh_path(root),
    }
}
/// An entry is installed only when its entire file set is present.
pub fn entry_installed_at(root: &Path, entry: &SherpaCatalogEntry) -> bool {
    let dir = entry_dir(root, entry);
    entry.files.iter().all(|f| dir.join(f.filename).is_file())
}
/// Display size of an entry: the sum of its catalog `bytes`.
pub(super) fn entry_bytes(entry: &SherpaCatalogEntry) -> u64 {
    entry.files.iter().map(|f| f.bytes).sum()
}

