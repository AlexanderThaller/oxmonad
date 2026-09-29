//! Merging the user's config over the embedded defaults.

use std::collections::BTreeMap;

use toml::{
    Table,
    Value,
};

use super::ConfigError;

/// The value that removes a default binding.
pub(super) const UNBIND: &str = "none";

/// Recursively merges `overlay` into `base`: nested tables merge, every
/// other value (including arrays) is replaced wholesale.
pub(super) fn deep_merge(base: &mut Table, overlay: Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(Value::Table(base_table)), Value::Table(overlay_table)) => {
                deep_merge(base_table, overlay_table);
            }
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

/// Removes a bindings table (`keybindings`/`pointer_bindings`) from a
/// document, as `(combo string, action string)` pairs.
pub(super) fn take_bindings(
    doc: &mut Table,
    section: &str,
) -> Result<Vec<(String, String)>, ConfigError> {
    let Some(value) = doc.remove(section) else {
        return Ok(Vec::new());
    };
    let Value::Table(table) = value else {
        return Err(ConfigError::InvalidSection(section.to_owned()));
    };
    table
        .into_iter()
        .map(|(combo, value)| {
            if let Value::String(action) = value {
                Ok((combo, action))
            } else {
                Err(ConfigError::InvalidBindingValue(combo))
            }
        })
        .collect()
}

/// Resolves default and user bindings into one map: user entries override
/// defaults that resolve to the same combination, and [`UNBIND`] removes
/// them.
pub(super) fn resolve_bindings<C: Ord, A>(
    defaults: Vec<(String, String)>,
    user: Vec<(String, String)>,
    mut resolve: impl FnMut(&str, &str) -> Result<(C, A), ConfigError>,
    mut resolve_combo: impl FnMut(&str) -> Result<C, ConfigError>,
) -> Result<BTreeMap<C, A>, ConfigError> {
    let mut map = BTreeMap::new();
    for (combo, action) in defaults.into_iter().chain(user) {
        if action.trim() == UNBIND {
            map.remove(&resolve_combo(&combo)?);
        } else {
            let (combo, action) = resolve(&combo, &action)?;
            map.insert(combo, action);
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_merge_tables_and_replace_arrays() {
        let mut base: Table = toml::from_str(
            r#"
            a = 1
            list = [1, 2, 3]
            [nested]
            x = "base"
            y = "base"
            [nested.deeper]
            z = true
            "#,
        )
        .expect("valid");
        let overlay: Table = toml::from_str(
            r#"
            list = [9]
            [nested]
            y = "user"
            [nested.deeper]
            w = 1
            "#,
        )
        .expect("valid");
        deep_merge(&mut base, overlay);
        assert_eq!(base["a"].as_integer(), Some(1));
        assert_eq!(base["list"].as_array().map(Vec::len), Some(1));
        assert_eq!(base["nested"]["x"].as_str(), Some("base"));
        assert_eq!(base["nested"]["y"].as_str(), Some("user"));
        assert_eq!(base["nested"]["deeper"]["z"].as_bool(), Some(true));
        assert_eq!(base["nested"]["deeper"]["w"].as_integer(), Some(1));
    }
}
