use serde::{Deserialize, Serialize};

/// Implements SQLite text storage and `as_str`/`TryFrom<&str>` for a lowercase enum.
macro_rules! text_enum {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }
        }

        impl TryFrom<&str> for $name {
            type Error = String;
            fn try_from(s: &str) -> Result<Self, String> {
                match s {
                    $($text => Ok(Self::$variant),)+
                    other => Err(format!("invalid {}: {other}", stringify!($name))),
                }
            }
        }

        impl sqlx::Type<sqlx::Sqlite> for $name {
            fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
                <str as sqlx::Type<sqlx::Sqlite>>::type_info()
            }
            fn compatible(ty: &sqlx::sqlite::SqliteTypeInfo) -> bool {
                <str as sqlx::Type<sqlx::Sqlite>>::compatible(ty)
            }
        }

        impl<'q> sqlx::Encode<'q, sqlx::Sqlite> for $name {
            fn encode_by_ref(
                &self,
                buf: &mut <sqlx::Sqlite as sqlx::Database>::ArgumentBuffer<'q>,
            ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
                <&str as sqlx::Encode<sqlx::Sqlite>>::encode_by_ref(&self.as_str(), buf)
            }
        }

        impl<'r> sqlx::Decode<'r, sqlx::Sqlite> for $name {
            fn decode(
                value: <sqlx::Sqlite as sqlx::Database>::ValueRef<'r>,
            ) -> Result<Self, sqlx::error::BoxDynError> {
                let s = <&str as sqlx::Decode<sqlx::Sqlite>>::decode(value)?;
                Ok(Self::try_from(s)?)
            }
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Video,
    Image,
}

text_enum!(MediaKind { Video => "video", Image => "image" });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Scan,
    Upload,
}

text_enum!(Source { Scan => "scan", Upload => "upload" });

pub const VIDEO_EXTS: [&str; 6] = ["mp4", "m4v", "mov", "webm", "mkv", "avi"];
pub const IMAGE_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];

/// Classifies a file by extension (case-insensitive, no dot) and confirms the
/// leading bytes agree on the broad type. `None` means unsupported or mismatched.
/// Heads shorter than 12 bytes are too small to sniff and trust the extension.
pub fn kind_for(extension: &str, head: &[u8]) -> Option<MediaKind> {
    let ext = extension.to_ascii_lowercase();
    let expected = if VIDEO_EXTS.contains(&ext.as_str()) {
        MediaKind::Video
    } else if IMAGE_EXTS.contains(&ext.as_str()) {
        MediaKind::Image
    } else {
        return None;
    };
    if head.len() < 12 && infer::get(head).is_none() {
        return Some(expected);
    }
    let matcher = infer::get(head)?.matcher_type();
    match (expected, matcher) {
        (MediaKind::Video, infer::MatcherType::Video) => Some(expected),
        (MediaKind::Image, infer::MatcherType::Image) => Some(expected),
        _ => None,
    }
}
