//! Reading YAML with the tag rule of the bridge that `A2-009` replaced (`A2-169`).
//!
//! `ayaml` keeps a tag outside YAML's core schema, such as `!!timestamp`, on the node it reads,
//! and hands a target that reads any value, such as a JSON value or a buffered `#[serde(flatten)]`
//! field, that node as an enum whose variant names the tag. The bridge, `yaml_serde`, treated only
//! a tag written with a local `!` as a tag and dropped every other one, so v0.6.0 read
//! `title: !!timestamp 2026-01-01` as the string beneath it, and `ayaml` refuses it.
//!
//! [`from_str`] reads as the bridge did. A global tag (`!!timestamp`, `!!binary`, a verbatim
//! `!<tag:example.com,2000:x>`, or a handle that a `%TAG` directive maps to a URI) is dropped,
//! and the value beneath it is read as the target asks, with one exception the bridge made: a
//! target that reads any value gets a scalar beneath the tag as its text, so `!e!x 1` is the
//! string `1` to a JSON value and the integer 1 to a `u32`. A local tag such as `!local`, which
//! the bridge kept and so refused where a target could not take a tag, is still refused, now with
//! an error that names it. A target that selects an enum variant by a local tag still does.
//! Everything else reaches the target exactly as `ayaml::compat` reads it.
//!
//! Two readings are not restored, both of documents no aplan format has a use for. Beneath a tag
//! in YAML's own namespace that is not a core one, such as `!!timestamp`, `ayaml` 0.1.3 keeps a
//! scalar as a string and refuses a mapping for a struct, so a number, a boolean or a struct
//! written there is refused, where the bridge parsed the text. Beneath any other global tag,
//! `ayaml` reads `~` as null for a unit target, where the bridge never read a tagged scalar as
//! null. A task's numbers are checked against its schema first, which refused such a value as a
//! string in v0.6.0 too.
//!
//! [`from_str_strict`] is the same rule for the formats aplan defines itself, which keep
//! `ayaml`'s strict reader.

use serde::de::{
    self, Deserialize, DeserializeOwned, DeserializeSeed, Deserializer, EnumAccess, MapAccess,
    SeqAccess, VariantAccess, Visitor,
};
use std::fmt;
use std::marker::PhantomData;

/// Deserialize one document with `ayaml::compat`, reading a value beneath a global tag as the
/// bridge did (see the module docs).
///
/// # Errors
/// When `ayaml::compat` refuses the input, the target refuses the value, or a local tag sits on a
/// value the target cannot read with a tag on it.
pub fn from_str<'de, T: Deserialize<'de>>(input: &'de str) -> Result<T, yaml_compat::Error> {
    yaml_compat::compat::from_str_seed(input, Seed(PhantomData::<T>))
}

/// Deserialize one document with `ayaml`'s strict reader, under the tag rule of [`from_str`].
///
/// The strict reader keeps its own refusals (a duplicate key, a flow collection whose
/// continuation line is not indented). A document it parses that carries a tag is then read by
/// [`from_str`]: `ayaml` 0.1.3 offers a seed entry point only on its compatibility reader, and
/// only a typed read keeps the text of a scalar beneath a dropped tag. Every other document, one
/// it refuses included, reads exactly as the strict reader reads it, with the same error.
///
/// # Errors
/// When the strict reader refuses the input, the target refuses the value, or a local tag sits on
/// a value the target cannot read with a tag on it.
pub fn from_str_strict<T: DeserializeOwned>(input: &str) -> Result<T, yaml_compat::Error> {
    // Every tag is written with `!`, so a document without one reads as the strict reader reads
    // it, without a second parse.
    if input.contains('!')
        && yaml_compat::from_str::<yaml_compat::Value>(input)
            .is_ok_and(|parsed| carries_tag(&parsed))
    {
        return from_str(input);
    }
    yaml_compat::from_str(input)
}

/// Whether a tag `ayaml` keeps sits anywhere in `value`, keys included.
fn carries_tag(value: &yaml_compat::Value) -> bool {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            yaml_compat::Value::Tagged(_) => return true,
            yaml_compat::Value::Sequence(items) => pending.extend(items),
            yaml_compat::Value::Mapping(mapping) => {
                for (key, value) in mapping.iter() {
                    pending.push(key);
                    pending.push(value);
                }
            }
            _ => {}
        }
    }
    false
}

/// The variant name `ayaml` gives a tagged node is a local tag's name after its `!`, or any other
/// tag's full name in angle brackets. A global tag's full name does not start with `!`; a local
/// tag that needs brackets (`!<!x>`) does.
fn is_global(variant: &str) -> bool {
    variant
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
        .is_some_and(|full| !full.starts_with('!'))
}

/// A local tag as it is written, from the variant name `ayaml` gives it.
fn written_local(variant: &str) -> String {
    match variant
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
    {
        Some(full) => format!("!<{full}>"),
        None => format!("!{variant}"),
    }
}

fn local_tag_refused<E: de::Error>(variant: &str) -> E {
    E::custom(format_args!(
        "the local tag `{}` cannot be read here: aplan reads YAML's core schema and drops a \
         global tag such as `!!timestamp`, but a local tag has no meaning to it; remove the tag",
        written_local(variant)
    ))
}

struct Seed<T>(PhantomData<T>);

impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for Seed<T> {
    type Value = T;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<T, D::Error> {
        T::deserialize(Tags(deserializer))
    }
}

/// A deserializer that applies the tag rule to every value beneath it.
struct Tags<D>(D);

/// The request a target made, kept so the value beneath a dropped tag is read with it too.
#[derive(Clone, Copy)]
enum Request {
    Any,
    Bool,
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    F32,
    F64,
    Char,
    Str,
    String,
    Bytes,
    ByteBuf,
    Option,
    Unit,
    UnitStruct(&'static str),
    NewtypeStruct(&'static str),
    Seq,
    Tuple(usize),
    TupleStruct(&'static str, usize),
    Map,
    Struct(&'static str, &'static [&'static str]),
    Enum(&'static str, &'static [&'static str]),
    Identifier,
    IgnoredAny,
}

impl Request {
    /// The request to make of the value beneath a dropped tag. The bridge read a scalar beneath
    /// a tag outside the core schema as its text where the target took any value, and never as
    /// null, so `!!timestamp ~` is the string `~`; a typed target got the type it asked for. So a
    /// request for any value, or for a unit, becomes a request for a string, which `ayaml`
    /// answers with a scalar's own text and with a collection as it is. Every other request
    /// stands.
    fn beneath_tag(self) -> Self {
        match self {
            Self::Any | Self::Unit | Self::UnitStruct(_) => Self::String,
            other => other,
        }
    }

    /// Make this request of `deserializer`, wrapping `visitor` so the rule reaches every value
    /// beneath it.
    fn make<'de, D: Deserializer<'de>, V: Visitor<'de>>(
        self,
        deserializer: D,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        let visitor = Visit {
            inner: visitor,
            request: self,
        };
        match self {
            Self::Any => deserializer.deserialize_any(visitor),
            Self::Bool => deserializer.deserialize_bool(visitor),
            Self::I8 => deserializer.deserialize_i8(visitor),
            Self::I16 => deserializer.deserialize_i16(visitor),
            Self::I32 => deserializer.deserialize_i32(visitor),
            Self::I64 => deserializer.deserialize_i64(visitor),
            Self::I128 => deserializer.deserialize_i128(visitor),
            Self::U8 => deserializer.deserialize_u8(visitor),
            Self::U16 => deserializer.deserialize_u16(visitor),
            Self::U32 => deserializer.deserialize_u32(visitor),
            Self::U64 => deserializer.deserialize_u64(visitor),
            Self::U128 => deserializer.deserialize_u128(visitor),
            Self::F32 => deserializer.deserialize_f32(visitor),
            Self::F64 => deserializer.deserialize_f64(visitor),
            Self::Char => deserializer.deserialize_char(visitor),
            Self::Str => deserializer.deserialize_str(visitor),
            Self::String => deserializer.deserialize_string(visitor),
            Self::Bytes => deserializer.deserialize_bytes(visitor),
            Self::ByteBuf => deserializer.deserialize_byte_buf(visitor),
            Self::Option => deserializer.deserialize_option(visitor),
            Self::Unit => deserializer.deserialize_unit(visitor),
            Self::UnitStruct(name) => deserializer.deserialize_unit_struct(name, visitor),
            Self::NewtypeStruct(name) => deserializer.deserialize_newtype_struct(name, visitor),
            Self::Seq => deserializer.deserialize_seq(visitor),
            Self::Tuple(len) => deserializer.deserialize_tuple(len, visitor),
            Self::TupleStruct(name, len) => {
                deserializer.deserialize_tuple_struct(name, len, visitor)
            }
            Self::Map => deserializer.deserialize_map(visitor),
            Self::Struct(name, fields) => deserializer.deserialize_struct(name, fields, visitor),
            Self::Enum(name, variants) => deserializer.deserialize_enum(name, variants, visitor),
            Self::Identifier => deserializer.deserialize_identifier(visitor),
            Self::IgnoredAny => deserializer.deserialize_ignored_any(visitor),
        }
    }
}

macro_rules! plain_requests {
    ($($method:ident => $request:ident,)*) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
                Request::$request.make(self.0, visitor)
            }
        )*
    };
}

impl<'de, D: Deserializer<'de>> Deserializer<'de> for Tags<D> {
    type Error = D::Error;

    plain_requests! {
        deserialize_any => Any,
        deserialize_bool => Bool,
        deserialize_i8 => I8,
        deserialize_i16 => I16,
        deserialize_i32 => I32,
        deserialize_i64 => I64,
        deserialize_i128 => I128,
        deserialize_u8 => U8,
        deserialize_u16 => U16,
        deserialize_u32 => U32,
        deserialize_u64 => U64,
        deserialize_u128 => U128,
        deserialize_f32 => F32,
        deserialize_f64 => F64,
        deserialize_char => Char,
        deserialize_str => Str,
        deserialize_string => String,
        deserialize_bytes => Bytes,
        deserialize_byte_buf => ByteBuf,
        deserialize_option => Option,
        deserialize_unit => Unit,
        deserialize_seq => Seq,
        deserialize_map => Map,
        deserialize_identifier => Identifier,
        deserialize_ignored_any => IgnoredAny,
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        Request::UnitStruct(name).make(self.0, visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        Request::NewtypeStruct(name).make(self.0, visitor)
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        Request::Tuple(len).make(self.0, visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        Request::TupleStruct(name, len).make(self.0, visitor)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        Request::Struct(name, fields).make(self.0, visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        Request::Enum(name, variants).make(self.0, visitor)
    }

    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }
}

/// A target's visitor, with the request it was made under.
struct Visit<V> {
    inner: V,
    request: Request,
}

macro_rules! forward_visits {
    ($($method:ident($ty:ty),)*) => {
        $(
            fn $method<E: de::Error>(self, value: $ty) -> Result<Self::Value, E> {
                self.inner.$method(value)
            }
        )*
    };
}

impl<'de, V: Visitor<'de>> Visitor<'de> for Visit<V> {
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.expecting(formatter)
    }

    forward_visits! {
        visit_bool(bool),
        visit_i8(i8),
        visit_i16(i16),
        visit_i32(i32),
        visit_i64(i64),
        visit_i128(i128),
        visit_u8(u8),
        visit_u16(u16),
        visit_u32(u32),
        visit_u64(u64),
        visit_u128(u128),
        visit_f32(f32),
        visit_f64(f64),
        visit_char(char),
        visit_str(&str),
        visit_borrowed_str(&'de str),
        visit_string(String),
        visit_bytes(&[u8]),
        visit_borrowed_bytes(&'de [u8]),
        visit_byte_buf(Vec<u8>),
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        self.inner.visit_none()
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        self.inner.visit_unit()
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        self.inner.visit_some(Tags(deserializer))
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        self.inner.visit_newtype_struct(Tags(deserializer))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
        self.inner.visit_seq(Elements(seq))
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        self.inner.visit_map(Entries(map))
    }

    /// `ayaml` presents a tagged node as an enum whose variant names the tag. A global tag is
    /// dropped and the value beneath it read with the target's own request; anything else
    /// reaches the target as it came.
    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Self::Value, A::Error> {
        let (variant, access) = data.variant_seed(VariantName)?;
        if is_global(&variant) {
            return access.newtype_variant_seed(Beneath {
                inner: self.inner,
                request: self.request,
            });
        }
        let enum_target = matches!(self.request, Request::Enum(..));
        let replay = Replay {
            variant: variant.clone(),
            access,
        };
        self.inner
            .visit_enum(replay)
            .map_err(|error| refusal(error, enum_target, &variant))
    }
}

/// The error for a target that refused a node under a local tag. An enum's own refusal names
/// the variant it did not know. Any other target that refuses has refused an enum, which says
/// nothing about the tag, so its refusal is replaced by one that names the tag.
fn refusal<E: de::Error>(error: E, enum_target: bool, variant: &str) -> E {
    if enum_target {
        error
    } else {
        local_tag_refused(variant)
    }
}

/// Reads an enum variant's name as a string, which is how `ayaml` names a tag.
struct VariantName;

impl<'de> DeserializeSeed<'de> for VariantName {
    type Value = String;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<String, D::Error> {
        String::deserialize(deserializer)
    }
}

/// The value beneath a dropped tag, read with the request the target made of the tagged node (see
/// [`Request::beneath_tag`]).
struct Beneath<V> {
    inner: V,
    request: Request,
}

impl<'de, V: Visitor<'de>> DeserializeSeed<'de> for Beneath<V> {
    type Value = V::Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<V::Value, D::Error> {
        // `make` wraps the visitor, which carries the rule to every value beneath; wrapping the
        // deserializer too would wrap the visitor again at each level.
        self.request.beneath_tag().make(deserializer, self.inner)
    }
}

/// An enum access whose variant name has been read, handed on with that name.
struct Replay<A> {
    variant: String,
    access: A,
}

impl<'de, A: VariantAccess<'de>> EnumAccess<'de> for Replay<A> {
    type Error = A::Error;
    type Variant = Variant<A>;

    fn variant_seed<S: DeserializeSeed<'de>>(
        self,
        seed: S,
    ) -> Result<(S::Value, Variant<A>), A::Error> {
        let variant =
            seed.deserialize(de::value::StringDeserializer::<A::Error>::new(self.variant))?;
        Ok((variant, Variant(self.access)))
    }
}

struct Variant<A>(A);

impl<'de, A: VariantAccess<'de>> VariantAccess<'de> for Variant<A> {
    type Error = A::Error;

    fn unit_variant(self) -> Result<(), A::Error> {
        self.0.unit_variant()
    }

    fn newtype_variant_seed<S: DeserializeSeed<'de>>(self, seed: S) -> Result<S::Value, A::Error> {
        self.0.newtype_variant_seed(Within(seed))
    }

    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, A::Error> {
        self.0.tuple_variant(
            len,
            Visit {
                inner: visitor,
                request: Request::Tuple(len),
            },
        )
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, A::Error> {
        self.0.struct_variant(
            fields,
            Visit {
                inner: visitor,
                request: Request::Map,
            },
        )
    }
}

/// A seed whose deserializer applies the tag rule.
struct Within<S>(S);

impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for Within<S> {
    type Value = S::Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<S::Value, D::Error> {
        self.0.deserialize(Tags(deserializer))
    }
}

struct Elements<A>(A);

impl<'de, A: SeqAccess<'de>> SeqAccess<'de> for Elements<A> {
    type Error = A::Error;

    fn next_element_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, A::Error> {
        self.0.next_element_seed(Within(seed))
    }

    fn size_hint(&self) -> Option<usize> {
        self.0.size_hint()
    }
}

struct Entries<A>(A);

impl<'de, A: MapAccess<'de>> MapAccess<'de> for Entries<A> {
    type Error = A::Error;

    fn next_key_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, A::Error> {
        self.0.next_key_seed(Within(seed))
    }

    fn next_value_seed<S: DeserializeSeed<'de>>(&mut self, seed: S) -> Result<S::Value, A::Error> {
        self.0.next_value_seed(Within(seed))
    }

    fn size_hint(&self) -> Option<usize> {
        self.0.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::{Tags, from_str, from_str_strict};
    use serde::Deserialize;
    use serde::de::{self, DeserializeOwned, Deserializer, IgnoredAny, Visitor};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::fmt::{self, Debug};

    /// A target that asks for bytes.
    #[derive(Debug, PartialEq)]
    struct Bytes(Vec<u8>);

    /// A target that asks for a byte buffer.
    #[derive(Debug, PartialEq)]
    struct ByteBuf(Vec<u8>);

    struct BytesVisitor;

    impl Visitor<'_> for BytesVisitor {
        type Value = Vec<u8>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("bytes")
        }

        fn visit_bytes<E: de::Error>(self, value: &[u8]) -> Result<Vec<u8>, E> {
            Ok(value.to_vec())
        }
    }

    impl<'de> Deserialize<'de> for Bytes {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserializer.deserialize_bytes(BytesVisitor).map(Self)
        }
    }

    impl<'de> Deserialize<'de> for ByteBuf {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserializer.deserialize_byte_buf(BytesVisitor).map(Self)
        }
    }

    /// A target that asks whether the format is human-readable, as address and time types do,
    /// and then skips the value.
    #[derive(Debug, PartialEq)]
    struct Readable(bool);

    impl<'de> Deserialize<'de> for Readable {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let readable = deserializer.is_human_readable();
            IgnoredAny::deserialize(deserializer).map(|_| Self(readable))
        }
    }

    /// A target that skips whatever it is given.
    #[derive(Debug, PartialEq)]
    struct Skipped;

    impl<'de> Deserialize<'de> for Skipped {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            IgnoredAny::deserialize(deserializer).map(|_| Self)
        }
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct Unit;

    #[derive(Deserialize, Debug, PartialEq)]
    struct Text(String);

    #[derive(Deserialize, Debug, PartialEq)]
    struct Count(u32);

    #[derive(Deserialize, Debug, PartialEq)]
    struct Pair(u8, u8);

    #[derive(Deserialize, Debug, PartialEq)]
    struct Known {
        known: u32,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    enum Shape {
        Empty,
        Circle(f64),
        Pair(u8, u8),
        Point { x: i8, y: i8 },
    }

    /// Reads one document as one target type.
    type ReadAs = dyn Fn(&str) -> Outcomes;

    /// Every request a target can make, each with a document written as `{tag}` and the
    /// value, read by `check` as `(kind, document)`.
    fn every_request(tag: &str, check: &mut dyn FnMut(&str, &str, &ReadAs)) {
        macro_rules! request {
            ($kind:literal, $ty:ty, $value:literal) => {
                check($kind, &format!("{tag}{}", $value), &|input| {
                    outcomes::<$ty>(input)
                })
            };
        }
        request!("bool", bool, "true");
        request!("i8", i8, "-5");
        request!("i16", i16, "-5");
        request!("i32", i32, "-5");
        request!("i64", i64, "-5");
        request!("i128", i128, "-5");
        request!("u8", u8, "5");
        request!("u16", u16, "5");
        request!("u32", u32, "5");
        request!("u64", u64, "5");
        request!("u128", u128, "5");
        request!("f32", f32, "1.5");
        request!("f64", f64, "1.5");
        request!("char", char, "x");
        request!("string", String, "5");
        request!("bytes", Bytes, "hi");
        request!("byte buffer", ByteBuf, "hi");
        request!("option", Option<u32>, "5");
        request!("option of null", Option<u32>, "~");
        request!("option of text", Option<String>, "~");
        request!("unit", (), "~");
        request!("unit struct", Unit, "~");
        request!("newtype of text", Text, "5");
        request!("newtype of a number", Count, "5");
        request!("tuple", (u8, String), "[1, x]");
        request!("tuple struct", Pair, "[1, 2]");
        request!("map", BTreeMap<String, u32>, "{a: 1}");
        request!("seq", Vec<String>, "[a, 5]");
        request!("struct", Known, "{known: 1}");
        request!("ignored", Skipped, "5");
        request!("enum", Shape, "Empty");
        request!("human-readable", Readable, "x");
        request!("any", serde_json::Value, "5");
        request!("any of null", serde_json::Value, "~");
    }

    /// One document read by this module, by `ayaml::compat`, and by the bridge, each as
    /// `Ok(value)` or `Err(message)`.
    struct Outcomes {
        ours: Result<String, String>,
        ayaml: Result<String, String>,
        bridge: Result<String, String>,
    }

    fn outcomes<T: DeserializeOwned + Debug>(input: &str) -> Outcomes {
        fn shown<T: Debug, E: fmt::Display>(read: Result<T, E>) -> Result<String, String> {
            read.map(|value| format!("{value:?}"))
                .map_err(|error| error.to_string())
        }
        Outcomes {
            ours: shown(from_str::<T>(input)),
            ayaml: shown(yaml_compat::compat::from_str::<T>(input)),
            bridge: shown(yaml_serde_baseline::from_str::<T>(input)),
        }
    }

    #[test]
    fn every_request_reads_an_untagged_document_exactly_as_ayaml_reads_it() {
        let mut checked = 0;
        every_request("", &mut |kind, input, read| {
            let read = read(input);
            assert_eq!(read.ours, read.ayaml, "{kind}: {input:?}");
            checked += 1;
        });
        // A refusal is the same refusal, message and place included.
        for (kind, input, read) in [
            ("u8", "300", outcomes::<u8> as fn(&str) -> Outcomes),
            ("struct", "[1]", outcomes::<Known>),
            ("struct", "5", outcomes::<Known>),
            ("tuple struct", "[1]", outcomes::<Pair>),
            ("char", "xy", outcomes::<char>),
            ("enum", "Square", outcomes::<Shape>),
            ("bytes", "[1]", outcomes::<Bytes>),
        ] {
            let read = read(input);
            assert!(read.ayaml.is_err(), "{kind}: {input:?} is refused");
            assert_eq!(read.ours, read.ayaml, "{kind}: {input:?}");
        }
        assert!(checked > 30, "every request was read");
    }

    #[test]
    fn every_request_reads_beneath_a_global_tag_as_the_bridge_did() {
        // Where this module and the bridge part beneath a global tag, as (tag, request); see
        // the module docs. Beneath a tag in YAML's own namespace that is not a core one, `ayaml`
        // keeps a scalar as a string and refuses a tagged mapping for a struct, where the bridge
        // parsed the text: every typed number or boolean, an option or a newtype of one, and a
        // struct.
        let own_namespace = [
            "bool",
            "i8",
            "i16",
            "i32",
            "i64",
            "i128",
            "u8",
            "u16",
            "u32",
            "u64",
            "u128",
            "f32",
            "f64",
            "option",
            "newtype of a number",
            "struct",
        ]
        .map(|kind| ("!!timestamp ", kind));
        // Beneath any other global tag, `ayaml` reads `~` as null for a unit target; the bridge
        // never read a tagged scalar as null.
        let other_global = ["!<tag:example.com,2000:x> ", "!e!x "]
            .into_iter()
            .flat_map(|tag| [(tag, "unit"), (tag, "unit struct")]);
        let mut known: Vec<(String, String)> = own_namespace
            .into_iter()
            .chain(other_global)
            .map(|(tag, kind)| (tag.to_owned(), kind.to_owned()))
            .collect();
        known.sort();
        let mut parted = BTreeMap::new();
        for tag in ["!!timestamp ", "!<tag:example.com,2000:x> ", "!e!x "] {
            let directive = if tag == "!e!x " {
                "%TAG !e! tag:example.com,2000:\n--- "
            } else {
                ""
            };
            every_request(tag, &mut |kind, input, read| {
                if kind.starts_with("byte") {
                    // The bridge reads no bytes, tagged or not.
                    return;
                }
                let read = read(&format!("{directive}{input}"));
                if read.ours.is_ok() != read.bridge.is_ok()
                    || (read.ours.is_ok() && read.ours != read.bridge)
                {
                    parted.insert(
                        (tag.to_owned(), kind.to_owned()),
                        format!("ours={:?} bridge={:?}", read.ours, read.bridge),
                    );
                }
            });
        }
        assert_eq!(
            parted.keys().cloned().collect::<Vec<_>>(),
            known,
            "the requests that part from the bridge changed: {parted:#?}"
        );
    }

    #[test]
    fn a_local_tag_selects_every_kind_of_variant_as_the_bridge_did() {
        for (input, expected) in [
            ("!Circle 2.5\n", Shape::Circle(2.5)),
            ("!Pair [1, 2]\n", Shape::Pair(1, 2)),
            ("!Point {x: 1, y: 2}\n", Shape::Point { x: 1, y: 2 }),
            ("!Empty ~\n", Shape::Empty),
        ] {
            assert_eq!(from_str::<Shape>(input).ok(), Some(expected), "{input:?}");
            let read = outcomes::<Shape>(input);
            assert_eq!(read.ours, read.bridge, "{input:?}");
        }
    }

    fn json(input: &str) -> serde_json::Value {
        from_str(input).unwrap_or_else(|error| panic!("{input:?} reads: {error}"))
    }

    #[test]
    fn a_global_tag_is_dropped_and_the_value_beneath_it_read() {
        for (input, expected) in [
            ("a: !!timestamp 2026-01-01\n", json!({"a": "2026-01-01"})),
            ("a: !!binary aGVsbG8=\n", json!({"a": "aGVsbG8="})),
            (
                "a: !<tag:example.com,2000:x> hello\n",
                json!({"a": "hello"}),
            ),
            (
                "%TAG ! tag:example.com,2000:\n---\na: !x hello\n",
                json!({"a": "hello"}),
            ),
            (
                "%TAG !e! tag:example.com,2000:\n---\na: !e!x 1\n",
                json!({"a": "1"}),
            ),
            ("a: !!timestamp true\n", json!({"a": "true"})),
            ("a: !!timestamp ~\n", json!({"a": "~"})),
            ("a: !<tag:example.com,2000:n> 0x1F\n", json!({"a": "0x1F"})),
            ("a: !!set {x, y}\n", json!({"a": {"x": null, "y": null}})),
            (
                "a: [!!timestamp 2026-01-01, 2]\n",
                json!({"a": ["2026-01-01", 2]}),
            ),
            ("!!timestamp 2026-01-01: x\n", json!({"2026-01-01": "x"})),
        ] {
            assert_eq!(json(input), expected, "{input:?}");
        }
    }

    #[test]
    fn the_value_beneath_a_dropped_tag_is_read_as_the_target_asked() {
        #[derive(Deserialize, Debug, PartialEq)]
        #[serde(rename_all = "lowercase")]
        enum Status {
            Todo,
            Done,
        }
        #[derive(Deserialize, Debug, PartialEq)]
        struct Typed {
            status: Status,
            when: Option<String>,
            note: String,
            items: Vec<u32>,
        }
        let typed: Typed = from_str(
            "status: !<tag:example.com,2000:s> todo\nwhen: !!timestamp 2026-01-01\n\
             note: !!timestamp 2026\nitems: !<tag:example.com,2000:l> [1, 2]\n",
        )
        .expect("reads");
        assert_eq!(
            typed,
            Typed {
                status: Status::Todo,
                when: Some("2026-01-01".to_owned()),
                note: "2026".to_owned(),
                items: vec![1, 2],
            }
        );
        let done: Status = from_str("done\n").expect("an untagged variant reads");
        assert_eq!(done, Status::Done);
    }

    #[test]
    fn a_buffered_field_reads_past_a_global_tag() {
        // `#[serde(flatten)]` buffers through `deserialize_any`, which `ayaml` answers with an
        // enum for a tagged node; the KEP importer reads its unmodelled fields this way.
        #[derive(Deserialize)]
        struct Flattened {
            title: String,
            #[serde(flatten)]
            extra: BTreeMap<String, serde_json::Value>,
        }
        let read: Flattened =
            from_str("title: t\ncreation-date: !!timestamp 2024-01-15\n").expect("reads");
        assert_eq!(read.title, "t");
        assert_eq!(read.extra["creation-date"], json!("2024-01-15"));
    }

    #[test]
    fn a_local_tag_is_refused_by_name_where_the_target_cannot_keep_it() {
        // A verbatim local tag is the same tag as the short form, and is named that way.
        // A local name that itself starts with `!` keeps its brackets.
        for (input, written) in [
            ("a: !custom 1\n", "!custom"),
            ("a: !<!x> 1\n", "!x"),
            ("a: !<!!x> 1\n", "!<!!x>"),
        ] {
            let error = from_str::<serde_json::Value>(input)
                .expect_err("a local tag cannot be read as JSON")
                .to_string();
            assert!(
                error.contains(&format!("the local tag `{written}` cannot be read here")),
                "{input:?}: {error}"
            );
            assert!(
                error.starts_with("a: ") && error.contains("line 1"),
                "the refusal keeps its field and its place: {error}"
            );
        }
    }

    #[test]
    fn a_target_that_keeps_tags_keeps_a_local_one_and_an_enum_still_selects_by_it() {
        #[derive(Deserialize, Debug, PartialEq)]
        enum Shape {
            Circle(f64),
        }

        let value: yaml_compat::Value = from_str("a: !custom 1\n").expect("a value keeps it");
        let tagged = value
            .as_mapping()
            .and_then(|mapping| mapping.get("a"))
            .expect("the key");
        assert!(
            matches!(tagged, yaml_compat::Value::Tagged(_)),
            "{tagged:?}"
        );

        let shape: Shape = from_str("!Circle 2.5\n").expect("a local tag names the variant");
        assert_eq!(shape, Shape::Circle(2.5));
    }

    #[test]
    fn an_untagged_document_reads_exactly_as_ayaml_reads_it() {
        for input in [
            "a: 1\nb: [x, 'y', \"z\"]\nc: {d: ~}\n",
            "title: Fix it! Now\n",
            "a: !!int 3\nb: !!str 4\n",
            "",
        ] {
            assert_eq!(
                from_str::<serde_json::Value>(input).ok(),
                yaml_compat::compat::from_str::<serde_json::Value>(input).ok(),
                "{input:?}"
            );
            assert_eq!(
                from_str_strict::<serde_json::Value>(input).ok(),
                yaml_compat::from_str::<serde_json::Value>(input).ok(),
                "{input:?}"
            );
        }
    }

    #[test]
    fn the_strict_reader_drops_a_global_tag_and_keeps_its_own_refusals() {
        for (input, expected) in [
            ("a: !!timestamp 2026-01-01\n", json!({"a": "2026-01-01"})),
            ("a: [x, !!timestamp 1]\n", json!({"a": ["x", "1"]})),
        ] {
            let read: serde_json::Value = from_str_strict(input).expect("reads past the tag");
            assert_eq!(read, expected, "{input:?}");
        }
        for (input, what) in [
            ("a: 1\na: 2\n", "a duplicate key"),
            ("a: !!timestamp 1\na: 2\n", "a duplicate key beside a tag"),
            ("a: [1,\n2]\n", "an unindented flow continuation"),
            (
                "a: [!!timestamp 1,\n2]\n",
                "an unindented flow continuation beside a tag",
            ),
        ] {
            let refused = from_str_strict::<serde_json::Value>(input)
                .expect_err(&format!("the strict reader still refuses {what}"))
                .to_string();
            let strict = yaml_compat::from_str::<serde_json::Value>(input)
                .expect_err("ayaml's strict reader refuses it")
                .to_string();
            assert_eq!(refused, strict, "{what} keeps the strict reader's error");
        }
        let error = from_str_strict::<serde_json::Value>("a: !custom 1\n")
            .expect_err("a local tag is refused")
            .to_string();
        assert!(
            error.contains("the local tag `!custom` cannot be read here")
                && error.contains("line 1"),
            "{error}"
        );
    }

    /// A deserializer that answers every request with the name of the method it was asked
    /// through and that method's arguments.
    struct Recorder;

    macro_rules! recorded {
        ($($method:ident($($arg:ident: $ty:ty),*);)*) => {
            $(
                fn $method<V: Visitor<'de>>(
                    self,
                    $($arg: $ty,)*
                    visitor: V,
                ) -> Result<V::Value, Self::Error> {
                    visitor.visit_str(&format!("{}{:?}", stringify!($method), ($($arg,)*)))
                }
            )*
        };
    }

    impl<'de> Deserializer<'de> for Recorder {
        type Error = de::value::Error;

        recorded! {
            deserialize_any();
            deserialize_bool();
            deserialize_i8();
            deserialize_i16();
            deserialize_i32();
            deserialize_i64();
            deserialize_i128();
            deserialize_u8();
            deserialize_u16();
            deserialize_u32();
            deserialize_u64();
            deserialize_u128();
            deserialize_f32();
            deserialize_f64();
            deserialize_char();
            deserialize_str();
            deserialize_string();
            deserialize_bytes();
            deserialize_byte_buf();
            deserialize_option();
            deserialize_unit();
            deserialize_unit_struct(name: &'static str);
            deserialize_newtype_struct(name: &'static str);
            deserialize_seq();
            deserialize_tuple(len: usize);
            deserialize_tuple_struct(name: &'static str, len: usize);
            deserialize_map();
            deserialize_struct(name: &'static str, fields: &'static [&'static str]);
            deserialize_enum(name: &'static str, variants: &'static [&'static str]);
            deserialize_identifier();
            deserialize_ignored_any();
        }
    }

    /// A visitor that takes the recorder's answer.
    struct Echo;

    impl Visitor<'_> for Echo {
        type Value = String;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("the method a request reached")
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<String, E> {
            Ok(value.to_owned())
        }
    }

    #[test]
    fn every_request_reaches_the_deserializer_method_it_names() {
        // The rule keeps each request to re-make it beneath a dropped tag, so a request that
        // reached another method would read that value as another type.
        let reached = [
            (Tags(Recorder).deserialize_any(Echo), "deserialize_any()"),
            (Tags(Recorder).deserialize_bool(Echo), "deserialize_bool()"),
            (Tags(Recorder).deserialize_i8(Echo), "deserialize_i8()"),
            (Tags(Recorder).deserialize_i16(Echo), "deserialize_i16()"),
            (Tags(Recorder).deserialize_i32(Echo), "deserialize_i32()"),
            (Tags(Recorder).deserialize_i64(Echo), "deserialize_i64()"),
            (Tags(Recorder).deserialize_i128(Echo), "deserialize_i128()"),
            (Tags(Recorder).deserialize_u8(Echo), "deserialize_u8()"),
            (Tags(Recorder).deserialize_u16(Echo), "deserialize_u16()"),
            (Tags(Recorder).deserialize_u32(Echo), "deserialize_u32()"),
            (Tags(Recorder).deserialize_u64(Echo), "deserialize_u64()"),
            (Tags(Recorder).deserialize_u128(Echo), "deserialize_u128()"),
            (Tags(Recorder).deserialize_f32(Echo), "deserialize_f32()"),
            (Tags(Recorder).deserialize_f64(Echo), "deserialize_f64()"),
            (Tags(Recorder).deserialize_char(Echo), "deserialize_char()"),
            (Tags(Recorder).deserialize_str(Echo), "deserialize_str()"),
            (
                Tags(Recorder).deserialize_string(Echo),
                "deserialize_string()",
            ),
            (
                Tags(Recorder).deserialize_bytes(Echo),
                "deserialize_bytes()",
            ),
            (
                Tags(Recorder).deserialize_byte_buf(Echo),
                "deserialize_byte_buf()",
            ),
            (
                Tags(Recorder).deserialize_option(Echo),
                "deserialize_option()",
            ),
            (Tags(Recorder).deserialize_unit(Echo), "deserialize_unit()"),
            (
                Tags(Recorder).deserialize_unit_struct("U", Echo),
                "deserialize_unit_struct(\"U\",)",
            ),
            (
                Tags(Recorder).deserialize_newtype_struct("N", Echo),
                "deserialize_newtype_struct(\"N\",)",
            ),
            (Tags(Recorder).deserialize_seq(Echo), "deserialize_seq()"),
            (
                Tags(Recorder).deserialize_tuple(2, Echo),
                "deserialize_tuple(2,)",
            ),
            (
                Tags(Recorder).deserialize_tuple_struct("T", 2, Echo),
                "deserialize_tuple_struct(\"T\", 2)",
            ),
            (Tags(Recorder).deserialize_map(Echo), "deserialize_map()"),
            (
                Tags(Recorder).deserialize_struct("S", &["a"], Echo),
                "deserialize_struct(\"S\", [\"a\"])",
            ),
            (
                Tags(Recorder).deserialize_enum("E", &["V"], Echo),
                "deserialize_enum(\"E\", [\"V\"])",
            ),
            (
                Tags(Recorder).deserialize_identifier(Echo),
                "deserialize_identifier()",
            ),
            (
                Tags(Recorder).deserialize_ignored_any(Echo),
                "deserialize_ignored_any()",
            ),
        ];
        for (reached, method) in reached {
            assert_eq!(reached.expect("the recorder answers"), method);
        }
        assert_eq!(
            (&Echo as &dyn de::Expected).to_string(),
            "the method a request reached"
        );
    }

    #[test]
    fn an_enum_keeps_its_own_refusal_of_a_local_tag_it_does_not_know() {
        let error = from_str::<Shape>("!Square 1\n")
            .expect_err("no variant is named `Square`")
            .to_string();
        assert!(error.contains("unknown variant `Square`"), "{error}");
        let read = outcomes::<Shape>("!Square 1\n");
        assert!(read.bridge.is_err(), "the bridge refused it too");
    }
}
