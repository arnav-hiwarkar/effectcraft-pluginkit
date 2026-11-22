use crate::effect::{Cx, Image};

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

#[macro_export]
macro_rules! params {
    ($(#[$meta:meta])* $vis:vis struct $name:ident { $($body:tt)* }) => {
        $crate::params!(@munch [$(#[$meta])* $vis struct $name] [] $($body)*);
    };
    (@munch $head:tt [$($done:tt)*] $group:literal { $($inner:tt)* } $(,)? $($rest:tt)*) => {
        $crate::params!(@group $head [$($done)*] $group [$($rest)*] $($inner)*);
    };
    (@munch $head:tt [$($done:tt)*] $fvis:vis $field:ident : $ty:ty = $kind:ident ( $($arg:tt)* ) $(, $($rest:tt)*)?) => {
        $crate::params!(@munch $head [$($done)* ($fvis, $field, $ty, stringify!($field), $kind, ($($arg)*))] $($($rest)*)?);
    };
    (@group $head:tt [$($done:tt)*] $group:literal [$($rest:tt)*] $fvis:vis $field:ident : $ty:ty = $kind:ident ( $($arg:tt)* ) $(, $($inner:tt)*)?) => {
        $crate::params!(@group $head [$($done)* ($fvis, $field, $ty, concat!($group, "/", stringify!($field)), $kind, ($($arg)*))] $group [$($rest)*] $($($inner)*)?);
    };
    (@group $head:tt [$($done:tt)*] $group:literal [$($rest:tt)*]) => {
        $crate::params!(@munch $head [$($done)*] $($rest)*);
    };
    (@munch [$(#[$meta:meta])* $vis:vis struct $name:ident] [$(($fvis:vis, $field:ident, $ty:ty, $id:expr, $kind:ident, ($($arg:tt)*)))*]) => {
        $(#[$meta])*
        $vis struct $name {
            $($fvis $field: $ty,)*
        }

        impl $name {
            pub const IDS: &'static [&'static str] = &[$($id),*];

            pub fn params() -> ::std::vec::Vec<$crate::Param> {
                ::std::vec![$($crate::Param::$kind($id, $($arg)*)),*]
            }

            pub fn read(cx: &$crate::Cx) -> Self {
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
        assert_eq!(Glow::IDS, ["radius", "Colour/tint", "Colour/mode", "on", "occluder"]);
        let spec = EffectSpec::new("glow", "Glow", "Stylize").params(Glow::params());
        assert_eq!(spec.params[1], Param::color("Colour/tint", "Tint", [1.0, 0.5, 0.25, 1.0]));
        assert_eq!(spec.params[2].kind, ParamKind::Popup { options: vec!["Add".into(), "Screen".into()], default: 1 });
        let params = BTreeMap::from([
            ("radius".to_string(), Value::Number(12.5)),
            ("Colour/tint".to_string(), Value::List(vec![0.25, 0.5, 0.75, 1.0])),
            ("Colour/mode".to_string(), Value::Number(1.0)),
            ("on".to_string(), Value::Bool(false)),
            ("occluder".to_string(), Value::Bool(true)),
        ]);
        let glow = Glow::read(&Cx::new(RenderCx { params, ..Default::default() }));
        assert_eq!((glow.radius, glow.tint, glow.mode, glow.on), (12.5, [0.25, 0.5, 0.75, 1.0], 1, false));
        assert_eq!(glow.occluder, crate::LayerParam { id: "occluder", connected: true });
    }
}
