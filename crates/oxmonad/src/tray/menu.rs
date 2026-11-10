//! `com.canonical.dbusmenu` layouts as [`MenuNode`] trees.

use std::collections::HashMap;

use zbus::{
    OwnedVariant,
    Variant,
};

use super::{
    MenuKind,
    MenuNode,
    Toggle,
};

/// The dbusmenu interface.
pub const INTERFACE: &str = "com.canonical.dbusmenu";

/// The body of a `GetLayout` reply: revision and `(ia{sv}av)` root.
pub type LayoutReply = (u32, (i32, HashMap<String, OwnedVariant>, Vec<OwnedVariant>));

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
pub fn unwrap_variant<'a>(mut value: &'a Variant<'a>) -> &'a Variant<'a> {
    while let Variant::Variant(inner) = value {
        value = inner;
    }
    value
}

/// Builds a node from its id, properties and child layouts.
fn node<'a>(
    id: i32,
    props: impl Iterator<Item = (&'a str, &'a Variant<'a>)>,
    children: impl Iterator<Item = &'a Variant<'a>>,
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
            ("label", Variant::Str(label)) => node.label = strip_mnemonics(label),
            ("enabled", Variant::Bool(enabled)) => node.enabled = *enabled,
            ("visible", Variant::Bool(visible)) => node.visible = *visible,
            ("type", Variant::Str(kind)) if kind.as_str() == "separator" => {
                node.kind = MenuKind::Separator;
            }
            ("toggle-type", Variant::Str(kind)) => toggle_type = kind.as_str(),
            ("toggle-state", Variant::I32(state)) => toggle_state = *state == 1,
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
pub fn parse_node(value: &Variant<'_>) -> Option<MenuNode> {
    let Variant::Structure(structure) = unwrap_variant(value) else {
        return None;
    };
    let [
        Variant::I32(id),
        Variant::Dict(props),
        Variant::Array(children),
    ] = structure.fields()
    else {
        return None;
    };
    let props = props.iter().filter_map(|(key, value)| {
        if let Variant::Str(key) = key {
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
    use zbus::{
        Array,
        Dict,
        Signature,
        Structure,
    };

    use super::*;

    /// A layout node value as a dbusmenu server sends it.
    pub fn layout_value(
        id: i32,
        props: &[(&str, Variant<'static>)],
        children: Vec<Variant<'static>>,
    ) -> Variant<'static> {
        let mut dict = Dict::new(&Signature::Str, &Signature::Variant);
        for (key, value) in props {
            dict.append(
                Variant::from((*key).to_owned()),
                Variant::Variant(Box::new(value.clone())),
            )
            .expect("dict entry");
        }
        let mut array = Array::new(&Signature::Variant);
        for child in children {
            array
                .append(Variant::Variant(Box::new(child)))
                .expect("array entry");
        }
        Variant::Structure(
            Structure::builder()
                .add_field(id)
                .append_field(Variant::Dict(dict))
                .append_field(Variant::Array(array))
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
                ("label", Variant::from("_Mute")),
                ("toggle-type", Variant::from("checkmark")),
                ("toggle-state", Variant::from(1i32)),
            ],
            vec![],
        );
        let separator = layout_value(3, &[("type", Variant::from("separator"))], vec![]);
        let radio = layout_value(
            5,
            &[
                ("label", Variant::from("Radio")),
                ("toggle-type", Variant::from("radio")),
                ("toggle-state", Variant::from(0i32)),
                ("enabled", Variant::from(false)),
            ],
            vec![],
        );
        let submenu = layout_value(
            4,
            &[
                ("label", Variant::from("More")),
                ("children-display", Variant::from("submenu")),
                ("visible", Variant::from(false)),
            ],
            vec![radio],
        );
        let reply: LayoutReply = (
            7,
            (
                0,
                HashMap::from([(
                    "children-display".to_owned(),
                    OwnedVariant::from(zbus::Str::from_static("submenu")),
                )]),
                vec![
                    OwnedVariant::try_from(check).expect("owned"),
                    OwnedVariant::try_from(separator).expect("owned"),
                    OwnedVariant::try_from(submenu).expect("owned"),
                    OwnedVariant::from(42i32),
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
