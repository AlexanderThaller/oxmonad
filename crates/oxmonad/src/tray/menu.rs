//! `com.canonical.dbusmenu` layouts as [`MenuNode`] trees.

use std::collections::HashMap;

use zbus::zvariant::{
    OwnedValue,
    Value,
};

use super::{
    MenuKind,
    MenuNode,
    Toggle,
};

/// The dbusmenu interface.
pub const INTERFACE: &str = "com.canonical.dbusmenu";

/// The body of a `GetLayout` reply: revision and `(ia{sv}av)` root.
pub type LayoutReply = (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>));

/// Removes GTK-style mnemonics: a single `_` goes, `__` becomes `_`.
#[must_use]
pub fn strip_mnemonics(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                chars.next();
                out.push('_');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `value` without any variant wrappers.
pub fn unwrap_variant<'a>(mut value: &'a Value<'a>) -> &'a Value<'a> {
    while let Value::Value(inner) = value {
        value = inner;
    }
    value
}

/// Builds a node from its id, properties and child layouts.
fn node<'a>(
    id: i32,
    props: impl Iterator<Item = (&'a str, &'a Value<'a>)>,
    children: impl Iterator<Item = &'a Value<'a>>,
) -> MenuNode {
    let mut node = MenuNode {
        id,
        label: String::new(),
        kind: MenuKind::Normal,
        toggle: Toggle::None,
        enabled: true,
        visible: true,
        children: children.filter_map(parse_node).collect(),
    };
    let mut toggle_type = "";
    let mut toggle_state = false;
    for (key, value) in props {
        match (key, unwrap_variant(value)) {
            ("label", Value::Str(label)) => node.label = strip_mnemonics(label),
            ("enabled", Value::Bool(enabled)) => node.enabled = *enabled,
            ("visible", Value::Bool(visible)) => node.visible = *visible,
            ("type", Value::Str(kind)) if kind.as_str() == "separator" => {
                node.kind = MenuKind::Separator;
            }
            ("toggle-type", Value::Str(kind)) => toggle_type = kind.as_str(),
            ("toggle-state", Value::I32(state)) => toggle_state = *state == 1,
            _ => {}
        }
    }
    node.toggle = match toggle_type {
        "checkmark" => Toggle::Check(toggle_state),
        "radio" => Toggle::Radio(toggle_state),
        _ => Toggle::None,
    };
    node
}

/// Parses one `(ia{sv}av)` layout value (possibly variant-wrapped).
#[must_use]
pub fn parse_node(value: &Value<'_>) -> Option<MenuNode> {
    let Value::Structure(structure) = unwrap_variant(value) else {
        return None;
    };
    let [Value::I32(id), Value::Dict(props), Value::Array(children)] = structure.fields() else {
        return None;
    };
    let props = props.iter().filter_map(|(key, value)| {
        if let Value::Str(key) = key {
            Some((key.as_str(), value))
        } else {
            None
        }
    });
    Some(node(*id, props, children.inner().iter()))
}

/// Parses the root of a `GetLayout` reply.
#[must_use]
pub fn parse_layout(reply: &LayoutReply) -> MenuNode {
    let (_revision, (id, props, children)) = reply;
    node(
        *id,
        props.iter().map(|(key, value)| (key.as_str(), &**value)),
        children.iter().map(|child| &**child),
    )
}

#[cfg(test)]
pub mod tests {
    use zbus::zvariant::{
        Array,
        Dict,
        Signature,
        StructureBuilder,
    };

    use super::*;

    /// A layout node value as a dbusmenu server sends it.
    pub fn layout_value(
        id: i32,
        props: &[(&str, Value<'static>)],
        children: Vec<Value<'static>>,
    ) -> Value<'static> {
        let mut dict = Dict::new(&Signature::Str, &Signature::Variant);
        for (key, value) in props {
            dict.append(
                Value::from((*key).to_owned()),
                Value::Value(Box::new(value.clone())),
            )
            .expect("dict entry");
        }
        let mut array = Array::new(&Signature::Variant);
        for child in children {
            array
                .append(Value::Value(Box::new(child)))
                .expect("array entry");
        }
        Value::Structure(
            StructureBuilder::new()
                .add_field(id)
                .append_field(Value::Dict(dict))
                .append_field(Value::Array(array))
                .build()
                .expect("structure"),
        )
    }

    #[test]
    fn strips_mnemonics() {
        assert_eq!(strip_mnemonics("_Quit"), "Quit");
        assert_eq!(strip_mnemonics("Save __as_ file"), "Save _as file");
        assert_eq!(strip_mnemonics("plain"), "plain");
        assert_eq!(strip_mnemonics("trailing_"), "trailing");
    }

    #[test]
    fn parses_layout() {
        let check = layout_value(
            2,
            &[
                ("label", Value::from("_Mute")),
                ("toggle-type", Value::from("checkmark")),
                ("toggle-state", Value::from(1i32)),
            ],
            vec![],
        );
        let separator = layout_value(3, &[("type", Value::from("separator"))], vec![]);
        let radio = layout_value(
            5,
            &[
                ("label", Value::from("Radio")),
                ("toggle-type", Value::from("radio")),
                ("toggle-state", Value::from(0i32)),
                ("enabled", Value::from(false)),
            ],
            vec![],
        );
        let submenu = layout_value(
            4,
            &[
                ("label", Value::from("More")),
                ("children-display", Value::from("submenu")),
                ("visible", Value::from(false)),
            ],
            vec![radio],
        );
        let reply: LayoutReply = (
            7,
            (
                0,
                HashMap::from([(
                    "children-display".to_owned(),
                    OwnedValue::from(zbus::zvariant::Str::from_static("submenu")),
                )]),
                vec![
                    OwnedValue::try_from(check).expect("owned"),
                    OwnedValue::try_from(separator).expect("owned"),
                    OwnedValue::try_from(submenu).expect("owned"),
                    OwnedValue::from(42i32),
                ],
            ),
        );
        let leaf = |id, label: &str| MenuNode {
            id,
            label: label.to_owned(),
            kind: MenuKind::Normal,
            toggle: Toggle::None,
            enabled: true,
            visible: true,
            children: vec![],
        };
        assert_eq!(
            parse_layout(&reply),
            MenuNode {
                children: vec![
                    MenuNode {
                        toggle: Toggle::Check(true),
                        ..leaf(2, "Mute")
                    },
                    MenuNode {
                        kind: MenuKind::Separator,
                        ..leaf(3, "")
                    },
                    MenuNode {
                        visible: false,
                        children: vec![MenuNode {
                            toggle: Toggle::Radio(false),
                            enabled: false,
                            ..leaf(5, "Radio")
                        }],
                        ..leaf(4, "More")
                    },
                ],
                ..leaf(0, "")
            }
        );
    }
}
