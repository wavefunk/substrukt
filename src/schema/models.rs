use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstruktMeta {
    pub title: String,
    pub slug: String,
    #[serde(default = "default_storage")]
    pub storage: StorageMode,
    #[serde(default)]
    pub kind: Kind,
    /// Which field to use as entry ID (for directory mode). Defaults to first string field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_field: Option<String>,
}

fn default_storage() -> StorageMode {
    StorageMode::Directory
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum StorageMode {
    Directory,
    SingleFile,
}

impl std::fmt::Display for StorageMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageMode::Directory => write!(f, "directory"),
            StorageMode::SingleFile => write!(f, "single-file"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Single,
    #[default]
    Collection,
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Kind::Single => write!(f, "single"),
            Kind::Collection => write!(f, "collection"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SchemaFile {
    pub meta: SubstruktMeta,
    pub schema: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_kind_single() {
        let json = r#"{"title":"Settings","slug":"settings","kind":"single"}"#;
        let meta: SubstruktMeta = serde_json::from_str(json).unwrap();
        assert_eq!(meta.kind, Kind::Single);
    }

    #[test]
    fn deserialize_kind_defaults_to_collection() {
        let json = r#"{"title":"Posts","slug":"posts"}"#;
        let meta: SubstruktMeta = serde_json::from_str(json).unwrap();
        assert_eq!(meta.kind, Kind::Collection);
    }

    #[test]
    fn deserialize_kind_collection_explicit() {
        let json = r#"{"title":"Posts","slug":"posts","kind":"collection"}"#;
        let meta: SubstruktMeta = serde_json::from_str(json).unwrap();
        assert_eq!(meta.kind, Kind::Collection);
    }
}
