//! Enums serialized by name in NBT (`StringRepresentable`) and by ordinal on the network.

/// Declares a `StringRepresentable` enum whose network id is its ordinal. Generates
/// `ALL`, `id`, `from_id`, `name`, `from_name`, `to_value`, `from_value`, `read` (ids out of
/// range decode as the first variant, as `ByIdMap.OutOfBoundsStrategy.ZERO` does) and `write`.
/// `impl_component` additionally makes it a [`crate::component::ComponentValue`].
macro_rules! string_enum {
    ($(#[$m:meta])* $vis:vis enum $name:ident { $($(#[$vm:meta])* $variant:ident = $str:literal),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        $vis enum $name {
            $($(#[$vm])* $variant,)*
        }

        #[allow(dead_code)]
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),*];

            pub fn id(self) -> i32 {
                self as i32
            }

            pub fn from_id(id: i32) -> Option<Self> {
                Self::ALL.get(usize::try_from(id).ok()?).copied()
            }

            pub fn name(self) -> &'static str {
                match self {
                    $($name::$variant => $str,)*
                }
            }

            pub fn from_name(s: &str) -> Option<Self> {
                match s {
                    $($str => Some($name::$variant),)*
                    _ => None,
                }
            }

            pub fn to_value(self) -> $crate::value::Value {
                $crate::value::Value::str(self.name())
            }

            pub fn from_value(v: &$crate::value::Value) -> $crate::value::DataResult<Self> {
                let s = v.as_str()?;
                Self::from_name(s).ok_or_else(|| $crate::value::DataError(format!("unknown {} {s:?}", stringify!($name))))
            }

            pub fn read(r: &mut kiln_proto::Reader<'_>) -> $crate::wire::WireResult<Self> {
                Ok(Self::from_id(r.varint()?).unwrap_or(Self::ALL[0]))
            }

            pub fn write(self, out: &mut bytes::BytesMut) {
                kiln_proto::WriteExt::put_varint(out, self.id());
            }
        }
    };
}

/// Implements `ComponentValue` for a [`string_enum!`] type.
macro_rules! impl_component_enum {
    ($($name:ident),* $(,)?) => {$(
        impl $crate::component::ComponentValue for $name {
            fn read(r: &mut kiln_proto::Reader<'_>) -> $crate::wire::WireResult<Self> {
                $name::read(r)
            }
            fn write(&self, out: &mut bytes::BytesMut) {
                $name::write(*self, out)
            }
            fn to_value(&self) -> $crate::value::Value {
                $name::to_value(*self)
            }
            fn from_value(v: &$crate::value::Value) -> $crate::value::DataResult<Self> {
                $name::from_value(v)
            }
        }
    )*};
}

pub(crate) use {impl_component_enum, string_enum};
