//! Diff two XFS filesystems, field by field.
//!
//! Both sides are read with [`crate::inspect::dump`]; every key present on
//! either side is compared. A difference is judged by the field's
//! [`Class`]: only [`Class::Structural`] differences mean the filesystems
//! differ — the UUID and the random or clock-derived values do not.

use std::collections::BTreeMap;

use crate::device::BlockDevice;
use crate::error::Result;
use crate::inspect::{dump, Class, Dump};

/// One field that differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    /// The field.
    pub key: String,
    /// Our value, or `None` if we lack the field.
    pub ours: Option<String>,
    /// Their value, or `None` if they lack it.
    pub theirs: Option<String>,
    /// How the difference is judged.
    pub class: Class,
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let show = |v: &Option<String>| v.clone().unwrap_or_else(|| "<absent>".into());
        write!(
            f,
            "{:?} {}: ours {} theirs {}",
            self.class,
            self.key,
            show(&self.ours),
            show(&self.theirs)
        )
    }
}

/// Every difference between two dumps, in key order.
pub fn diff(ours: &Dump, theirs: &Dump) -> Vec<Difference> {
    let a: BTreeMap<&str, (&str, Class)> =
        ours.fields.iter().map(|f| (f.key.as_str(), (f.value.as_str(), f.class))).collect();
    let b: BTreeMap<&str, (&str, Class)> =
        theirs.fields.iter().map(|f| (f.key.as_str(), (f.value.as_str(), f.class))).collect();
    let mut keys: Vec<&str> = a.keys().chain(b.keys()).copied().collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .filter_map(|k| {
            let (x, y) = (a.get(k), b.get(k));
            if x.map(|v| v.0) == y.map(|v| v.0) {
                return None;
            }
            // A field one side lacks is structural whatever it holds.
            let class = match (x, y) {
                (Some(v), Some(_)) => v.1,
                _ => Class::Structural,
            };
            Some(Difference {
                key: k.to_string(),
                ours: x.map(|v| v.0.to_string()),
                theirs: y.map(|v| v.0.to_string()),
                class,
            })
        })
        .collect()
}

/// Dump both devices and diff them.
pub async fn compare<A, B>(ours: &A, theirs: &B) -> Result<Vec<Difference>>
where
    A: BlockDevice + ?Sized,
    B: BlockDevice + ?Sized,
{
    Ok(diff(&dump(ours).await?, &dump(theirs).await?))
}

/// Only the differences that matter.
pub fn structural(diffs: &[Difference]) -> Vec<&Difference> {
    diffs.iter().filter(|d| d.class == Class::Structural).collect()
}
