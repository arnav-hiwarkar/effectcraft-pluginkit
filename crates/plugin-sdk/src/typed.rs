use crate::effect::{Cx, Image};
use crate::wire::{Param, ParamKind, Value};

pub trait FromParam: Sized {
    fn from_param(cx: &Cx, id: &'static str) -> Self;
}

impl FromParam for f64 {
    fn from_param(cx: &Cx, id: &'static str) -> Self {
        cx.f(id)
    }
}

impl FromParam for f32 {
    fn from_param(cx: &Cx, id: &'static str) -> Self {
        cx.f(id) as f32
    }
}

impl FromParam for bool {
    fn from_param(cx: &Cx, id: &'static str) -> Self {
        cx.bool(id)
    }
}

impl FromParam for usize {
    fn from_param(cx: &Cx, id: &'static str) -> Self {
        cx.index(id)
    }
}

impl FromParam for u32 {
    fn from_param(cx: &Cx, id: &'static str) -> Self {
        u32::try_from(cx.index(id)).unwrap_or(u32::MAX)
    }
}

impl FromParam for [f64; 2] {
    fn from_param(cx: &Cx, id: &'static str) -> Self {
        cx.point(id)
    }
}

impl FromParam for [f64; 3] {
    fn from_param(cx: &Cx, id: &'static str) -> Self {
        cx.point3(id)
    }
}

impl FromParam for [f32; 4] {
    fn from_param(cx: &Cx, id: &'static str) -> Self {
        cx.color(id)
    }
}

impl FromParam for String {
    fn from_param(cx: &Cx, id: &'static str) -> Self {
        cx.text(id).to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerParam {
    pub id: &'static str,
    pub connected: bool,
}

impl LayerParam {
    pub fn image(&self, cx: &Cx) -> Option<Image> {
        self.connected.then(|| cx.layer(self.id)).flatten()
    }

    pub fn image_at(&self, cx: &Cx, comp_time: f64) -> Option<Image> {
        self.connected.then(|| cx.layer_at(self.id, comp_time)).flatten()
    }
}

impl FromParam for LayerParam {
    fn from_param(cx: &Cx, id: &'static str) -> Self {
        LayerParam { id, connected: cx.bool(id) }
    }
}

impl Param {
    pub fn fit(&self, value: Value) -> Result<Value, String> {
        let list = |items: &[f64], n: usize| (items.len() == n).then(|| Value::List(items.to_vec()));
        let fitted = match (&self.kind, &value) {
            (ParamKind::Slider { min, max, .. }, Value::Number(n)) => Some(Value::Number(n.max(*min).min(*max))),
            (ParamKind::Percent { .. } | ParamKind::Angle { .. }, Value::Number(n)) => Some(Value::Number(*n)),
            (ParamKind::Popup { .. } | ParamKind::Mask, Value::Number(n)) => Some(Value::Number(n.max(0.0).trunc())),
            (ParamKind::Checkbox { .. } | ParamKind::Layer, Value::Bool(b)) => Some(Value::Bool(*b)),
            (ParamKind::Point { .. }, Value::List(items)) => list(items, 2),
            (ParamKind::Point3 { .. }, Value::List(items)) => list(items, 3),
            (ParamKind::Color { .. }, Value::List(items)) => list(items, 4),
            (ParamKind::Text { .. } | ParamKind::Custom { .. }, Value::Text(text)) => Some(Value::Text(text.clone())),
            _ => None,
        };
        fitted.ok_or_else(|| {
            let wanted = match &self.kind {
                ParamKind::Slider { .. } | ParamKind::Percent { .. } | ParamKind::Angle { .. } => "a number",
                ParamKind::Popup { .. } | ParamKind::Mask => "an index",
                ParamKind::Checkbox { .. } | ParamKind::Layer => "true or false",
                ParamKind::Point { .. } => "a list of 2 numbers",
                ParamKind::Point3 { .. } => "a list of 3 numbers",
                ParamKind::Color { .. } => "a list of 4 numbers",
                ParamKind::Text { .. } | ParamKind::Custom { .. } => "text",
            };
            let given = match value {
                Value::Number(n) => format!("the number {n}"),
                Value::Bool(b) => format!("{b}"),
                Value::Text(text) => format!("the text {text:?}"),
                Value::List(items) => format!("a list of {} numbers", items.len()),
            };
            format!("parameter `{}` takes {wanted}, not {given}", self.id)
        })
    }
}

pub mod fits {
    #[diagnostic::on_unimplemented(
        message = "a slider, percent or angle parameter is read as a number, so its field cannot be `{Self}`",
        label = "declare this field as `f64` or `f32`"
    )]
    pub trait Number {}
    impl Number for f64 {}
    impl Number for f32 {}

    #[diagnostic::on_unimplemented(message = "a checkbox parameter is read as `bool`, so its field cannot be `{Self}`", label = "declare this field as `bool`")]
    pub trait Flag {}
    impl Flag for bool {}

    #[diagnostic::on_unimplemented(
        message = "a popup or mask parameter is read as an index, so its field cannot be `{Self}`",
        label = "declare this field as `usize` or `u32`"
    )]
    pub trait Choice {}
    impl Choice for usize {}
    impl Choice for u32 {}

    #[diagnostic::on_unimplemented(
        message = "a point parameter is read as `[f64; 2]`, so its field cannot be `{Self}`",
        label = "declare this field as `[f64; 2]`"
    )]
    pub trait Point {}
    impl Point for [f64; 2] {}

    #[diagnostic::on_unimplemented(
        message = "a point3 parameter is read as `[f64; 3]`, so its field cannot be `{Self}`",
        label = "declare this field as `[f64; 3]`"
    )]
    pub trait Point3 {}
    impl Point3 for [f64; 3] {}

    #[diagnostic::on_unimplemented(
        message = "a color parameter is read as `[f32; 4]`, so its field cannot be `{Self}`",
        label = "declare this field as `[f32; 4]`"
    )]
    pub trait Colour {}
    impl Colour for [f32; 4] {}

    #[diagnostic::on_unimplemented(
        message = "a text or custom parameter is read as `String`, so its field cannot be `{Self}`",
        label = "declare this field as `String`"
    )]
    pub trait Text {}
    impl Text for String {}

    #[diagnostic::on_unimplemented(
        message = "a layer parameter is read as `LayerParam`, so its field cannot be `{Self}`",
        label = "declare this field as `LayerParam`"
    )]
    pub trait Layer {}
    impl Layer for super::LayerParam {}

    pub fn slider<T: Number>() {}
    pub fn percent<T: Number>() {}
    pub fn angle<T: Number>() {}
    pub fn checkbox<T: Flag>() {}
    pub fn popup<T: Choice>() {}
    pub fn mask<T: Choice>() {}
    pub fn point<T: Point>() {}
    pub fn point3<T: Point3>() {}
    pub fn color<T: Colour>() {}
    pub fn text<T: Text>() {}
    pub fn custom<T: Text>() {}
    pub fn layer<T: Layer>() {}
}

/// Declares an effect's parameters and a struct that reads them.
///
/// ```compile_fail,E0277
/// craft_plugin::params! { struct Wrong { name: String = slider("Name", 1.0, 0.0..=2.0) } }
/// ```
#[macro_export]
macro_rules! params {
    ($(#[$meta:meta])* $vis:vis struct $name:ident { $($body:tt)* }) => {
        $crate::params!(@munch [$(#[$meta])* $vis struct $name] [] $($body)*);
    };
    (@id $field:ident) => { ::core::stringify!($field) };
    (@id $field:ident $id:literal) => { $id };
    (@munch $head:tt [$($done:tt)*] $group:literal { $($inner:tt)* } , $($rest:tt)*) => {
        $crate::params!(@group $head [$($done)*] $group [$($rest)*] $($inner)*);
    };
    (@munch $head:tt [$($done:tt)*] $group:literal { $($inner:tt)* } $($rest:tt)*) => {
        $crate::params!(@group $head [$($done)*] $group [$($rest)*] $($inner)*);
    };
    (@munch $head:tt [$($done:tt)*] $fvis:vis $field:ident $(as $id:literal)? : $ty:ty = $kind:ident ( $($arg:tt)* ) $(. $method:ident ( $($method_arg:tt)* ))* $(, $($rest:tt)*)?) => {
        $crate::params!(@munch $head [$($done)* ($fvis, $field, $ty, $crate::params!(@id $field $($id)?), $kind, ($($arg)*), ($(. $method ( $($method_arg)* ))*))] $($($rest)*)?);
    };
    (@group $head:tt [$($done:tt)*] $group:literal [$($rest:tt)*] $fvis:vis $field:ident $(as $id:literal)? : $ty:ty = $kind:ident ( $($arg:tt)* ) $(. $method:ident ( $($method_arg:tt)* ))* $(, $($inner:tt)*)?) => {
        $crate::params!(@group $head [$($done)* ($fvis, $field, $ty, ::core::concat!($group, "/", $crate::params!(@id $field $($id)?)), $kind, ($($arg)*), ($(. $method ( $($method_arg)* ))*))] $group [$($rest)*] $($($inner)*)?);
    };
    (@group $head:tt [$($done:tt)*] $group:literal [$($rest:tt)*]) => {
        $crate::params!(@munch $head [$($done)*] $($rest)*);
    };
    (@munch [$(#[$meta:meta])* $vis:vis struct $name:ident] [$(($fvis:vis, $field:ident, $ty:ty, $id:expr, $kind:ident, ($($arg:tt)*), ($(. $method:ident ( $($method_arg:tt)* ))*)))*]) => {
        $(#[$meta])*
        $vis struct $name {
            $($fvis $field: $ty,)*
        }

        impl $name {
            pub const IDS: &'static [&'static str] = &[$($id),*];

            pub fn params() -> ::std::vec::Vec<$crate::Param> {
                ::std::vec![$($crate::Param::$kind($id, $($arg)*) $(. $method ( $($method_arg)* ))*),*]
            }

            pub fn read(cx: &$crate::Cx) -> Self {
                $($crate::typed::fits::$kind::<$ty>();)*
                let _ = cx;
                $name { $($field: <$ty as $crate::FromParam>::from_param(cx, $id),)* }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{Cx, EffectSpec, Param, ParamKind, RenderCx, Value};

    const MODES: [&str; 2] = ["Add", "Screen"];

    params! {
        struct Glow {
            radius: f64 = slider("Radius", 20.0, 0.0..=200.0),
            gain: f64 = slider("Gain", 1.0, 0.0..=1000.0).soft(0.01..=10.0).log(),
            "Colour" {
                tint: [f32; 4] = color("Tint", [1.0, 0.5, 0.25, 1.0]),
                mode: usize = popup("Mode", &MODES, 1),
            }
            on: bool = checkbox("Enabled", true),
            occluder: crate::LayerParam = layer("Occluder"),
        }
    }

    #[test]
    fn one_declaration_gives_the_parameter_list_with_group_ids_and_a_struct_read_by_those_ids() {
        assert_eq!(Glow::IDS, ["radius", "gain", "Colour/tint", "Colour/mode", "on", "occluder"]);
        let spec = EffectSpec::new("glow", "Glow", "Stylize").params(Glow::params());
        assert_eq!(spec.params[1], Param::slider("gain", "Gain", 1.0, 0.0..=1000.0).soft(0.01..=10.0).log());
        assert_eq!(spec.params[2], Param::color("Colour/tint", "Tint", [1.0, 0.5, 0.25, 1.0]));
        assert_eq!(spec.params[3].kind, ParamKind::Popup { options: vec!["Add".into(), "Screen".into()], default: 1 });
        let params = BTreeMap::from([
            ("radius".to_string(), Value::Number(12.5)),
            ("gain".to_string(), Value::Number(3.0)),
            ("Colour/tint".to_string(), Value::List(vec![0.25, 0.5, 0.75, 1.0])),
            ("Colour/mode".to_string(), Value::Number(1.0)),
            ("on".to_string(), Value::Bool(false)),
            ("occluder".to_string(), Value::Bool(true)),
        ]);
        let glow = Glow::read(&Cx::new(RenderCx { params, ..Default::default() }));
        assert_eq!((glow.radius, glow.gain, glow.tint, glow.mode, glow.on), (12.5, 3.0, [0.25, 0.5, 0.75, 1.0], 1, false));
        assert_eq!(glow.occluder, crate::LayerParam { id: "occluder", connected: true });
    }

    params! {
        struct Nothing {}
    }

    params! {
        struct Saved {
            kind as "type": usize = popup("Type", &MODES, 1),
            "Core" {
                core_gain as "gain": f64 = slider("Core Gain", 1.0, 0.0..=4.0),
                shape as "type": usize = popup("Shape", &MODES, 0),
            },
            "Glow" {
                glow_gain as "gain": f64 = slider("Glow Gain", 2.0, 0.0..=4.0),
            }
            "Empty" {}
            last: bool = checkbox("Last", true),
        }
    }

    #[test]
    fn a_field_states_its_saved_id_and_empty_structs_groups_and_trailing_commas_parse() {
        assert!(Nothing::IDS.is_empty() && Nothing::params().is_empty());
        let _ = Nothing::read(&Cx::new(RenderCx::default()));
        assert_eq!(Saved::IDS, ["type", "Core/gain", "Core/type", "Glow/gain", "last"]);
        assert_eq!(Saved::params()[3], Param::slider("Glow/gain", "Glow Gain", 2.0, 0.0..=4.0));
        let params = BTreeMap::from([
            ("type".to_string(), Value::Number(0.0)),
            ("Core/gain".to_string(), Value::Number(3.0)),
            ("Core/type".to_string(), Value::Number(1.0)),
            ("Glow/gain".to_string(), Value::Number(0.5)),
            ("last".to_string(), Value::Bool(false)),
        ]);
        let saved = Saved::read(&Cx::new(RenderCx { params, ..Default::default() }));
        assert_eq!((saved.kind, saved.core_gain, saved.shape, saved.glow_gain, saved.last), (0, 3.0, 1, 0.5, false));
    }
}
