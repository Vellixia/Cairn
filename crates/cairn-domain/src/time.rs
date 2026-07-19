//! Timezone-unambiguous timestamps: RFC 3339 UTC, millisecond precision.

use std::fmt;

use chrono::{DateTime, SecondsFormat, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Timestamp(#[schemars(with = "String")] pub DateTime<Utc>);

impl Timestamp {
    pub fn now() -> Self {
        // Truncate to millisecond precision so round-tripping through RFC 3339
        // strings is lossless.
        let now = Utc::now();
        let millis = now.timestamp_millis();
        Self(DateTime::from_timestamp_millis(millis).expect("valid timestamp"))
    }

    pub fn to_rfc3339(&self) -> String {
        self.0.to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    pub fn parse(s: &str) -> Result<Self, chrono::ParseError> {
        Ok(Self(DateTime::parse_from_rfc3339(s)?.with_timezone(&Utc)))
    }

    pub fn plus_seconds(&self, secs: i64) -> Self {
        Self(self.0 + chrono::Duration::seconds(secs))
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}
