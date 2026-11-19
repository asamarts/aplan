//! Reading YAML with the tag rule of the bridge that `A2-009` replaced (`A2-169`, `A2-179`).
//!
//! `ayaml` keeps a tag outside YAML's core schema, such as `!!timestamp`, on the node it reads,
//! and hands a target that reads any value, such as a JSON value or a buffered `#[serde(flatten)]`
//! field, that node as an enum whose variant names the tag. The bridge, `yaml_serde`, kept only a
//! tag written with a single `!` and dropped every other one: where a target read any value it
//! honoured a core tag and read a scalar beneath any other global tag as its text, and a typed
//! target got the type it asked for. So v0.6.0 read `title: !!timestamp 2026-01-01` as the string
//! beneath the tag, and `ayaml` refuses it.
//!
//! [`from_str`] reads as the bridge did. A global tag (`!!timestamp`, `!!binary`, a verbatim
//! `!<tag:example.com,2000:x>`, or a handle that a `%TAG` directive maps to a URI) is dropped, and
//! the value beneath it is read with the target's own request, except that a target reading any
//! value, or a unit, gets a scalar beneath the tag as its text, as the bridge gave it: `!e!x 1` is
//! the string `1` to a JSON value and the integer 1 to a `u32`, and `!!timestamp ~` is not null. A
//! local tag such as `!local`, which the bridge kept and so refused where a target read any value,
//! is still refused there, now with an error that names it; a typed target reads past it, as both
//! parsers do, and an enum selects a variant by it. An enum is asked for as any value (see the
//! private `EnumProbe`), so that only a tag is taken for one. Everything else reaches the target
//! exactly as `ayaml::compat` reads it.
//!
//! Three readings part from the bridge, all of documents no aplan format has a use for. Beneath a
//! tag in YAML's own namespace that is not a core one, such as `!!timestamp`, `ayaml` 0.1.3 keeps a
//! scalar as text and refuses a mapping for a struct, so a number, a boolean or a struct written
//! there is refused, where the bridge parsed the text; the refusal of a scalar names the tag.
//! Beneath any other global tag, and beneath a local tag, `ayaml` reads `~` as null for a unit
//! target, which the bridge refused. And an error inside the value of a variant named by a string
//! or a local tag does not name the variant in its path, as `ayaml` alone does: asked for any
//! value, it has no key to add. A task's numbers are checked against its schema first, which
//! refused a string in v0.6.0 too.
//!
//! [`from_str_strict`] is the same rule for the formats aplan defines itself, which keep
//! `ayaml`'s strict reader and its grammar.

use serde::de::{
    self, Deserialize, DeserializeOwned, DeserializeSeed, Deserializer, EnumAccess, MapAccess,
    SeqAccess, VariantAccess, Visitor,
};
use std::fmt::{self, Write as _};
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
/// A document without a tag `ayaml` keeps reads exactly as the strict reader reads it, a refused
/// one with the same error. One that carries such a tag is read again through the strict
/// reader's document cursor, which takes a seed, so the rule applies without changing the
/// grammar: a duplicate key, a flow collection whose continuation line is not indented, and every
/// other document the strict reader refuses is refused either way.
///
/// # Errors
/// When the strict reader refuses the input, the target refuses the value, or a local tag sits on
/// a value the target cannot read with a tag on it.
pub fn from_str_strict<T: DeserializeOwned>(input: &str) -> Result<T, yaml_compat::Error> {
    // Every tag is written with `!`, so a document without one reads as the strict reader reads
    // it, without a second parse. The first parse also refuses whatever the strict reader
    // refuses, a second document included, so the cursor reads the one document there is.
    if input.contains('!')
        && yaml_compat::from_str::<yaml_compat::Value>(input)
            .is_ok_and(|parsed| carries_tag(&parsed))
        && let Some(read) = yaml_compat::Deserializer::from_str(input)?
            .next_seed_borrowed(Seed(PhantomData::<T>))?
    {
        return Ok(read);
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

/// The name `ayaml` gives a tag when it hands a target a tagged node as an enum: a local tag's
/// name after its `!`, or any other tag's full name in angle brackets. A global tag's full name
/// does not start with `!`; a local name that is empty or starts with `!` or `<` is bracketed too,
/// as `<!name>`, so that every name reads back.
fn is_global(tag: &str) -> bool {
    tag.strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
        .is_some_and(|full| !full.starts_with('!'))
}

/// Whether a global tag is in YAML's own namespace, as `!!timestamp` is. `ayaml` reads a typed
/// value past any other global tag itself, so only these reach a typed target as a tag.
fn in_yaml_namespace(tag: &str) -> bool {
    tag.starts_with("<tag:yaml.org,2002:")
}

/// A local tag's name, which is the enum variant it selects, from the name `ayaml` gives the tag.
/// The non-specific tag `!` has the empty name.
fn local_name(tag: &str) -> &str {
    tag.strip_prefix("<!")
        .and_then(|rest| rest.strip_suffix('>'))
        .unwrap_or(tag)
}

/// A global tag as it is written: `!!name` in YAML's own namespace, verbatim otherwise.
fn written_global(tag: &str) -> String {
    let full = tag
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
        .unwrap_or(tag);
    match full.strip_prefix("tag:yaml.org,2002:") {
        Some(name) => format!("!!{name}"),
        None => format!("!<{full}>"),
    }
}

/// A local tag as it can be written back: `!name` when its name is made of tag characters,
/// verbatim as `!<!name>` when it is made of URI characters, and with every other byte escaped
/// otherwise, so `!%3Cx%3E` is written as it was and never as a verbatim tag YAML refuses.
fn written_local(name: &str) -> String {
    // YAML 1.2.2 `ns-tag-char` is `ns-uri-char` without `!` and the flow indicators. A `%`
    // starts an escape, so a name with one in it is escaped too.
    let tag_char = |byte: u8| byte.is_ascii_alphanumeric() || b"-#;/?:@&=+$_.~*'()".contains(&byte);
    let uri_char = |byte: u8| tag_char(byte) || b"!,[]".contains(&byte);
    if name.bytes().all(tag_char) {
        format!("!{name}")
    } else if name.bytes().all(uri_char) {
        format!("!<!{name}>")
    } else {
        name.bytes().fold(String::from("!"), |mut written, byte| {
            if tag_char(byte) {
                written.push(char::from(byte));
            } else {
                // Writing to a `String` cannot fail.
                let _ = write!(written, "%{byte:02X}");
            }
            written
        })
    }
}

fn local_tag_refused<E: de::Error>(tag: &str) -> E {
    let name = local_name(tag);
    let which = if name.is_empty() {
        "the non-specific tag `!`".to_owned()
    } else {
        format!("the local tag `{}`", written_local(name))
    };
    E::custom(format_args!(
        "{which} cannot be read here: aplan reads YAML's core schema and drops a global tag \
         such as `!!timestamp`, but a tag written with a single `!` has no meaning to it; remove \
         the tag"
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
    /// The request to make of the value beneath a dropped tag. Where a target took any value,
    /// the bridge read a scalar beneath a global tag outside the core schema as its text, so
    /// `!!timestamp ~` is the string `~`, and it read no unit there; a typed target got the type it
    /// asked for. So a request for any value, or for a unit, becomes a request for a string,
    /// which `ayaml` answers with a scalar's own text and with a collection as it is. Every other
    /// request stands.
    fn beneath_tag(self) -> Self {
        match self {
            Self::Any | Self::Unit | Self::UnitStruct(_) => Self::String,
            other => other,
        }
    }

    /// Whether this asks for a scalar other than text: a boolean, a number, or a unit.
    fn is_scalar(self) -> bool {
        matches!(
            self,
            Self::Bool
                | Self::I8
                | Self::I16
                | Self::I32
                | Self::I64
                | Self::I128
                | Self::U8
                | Self::U16
                | Self::U32
                | Self::U64
                | Self::U128
                | Self::F32
                | Self::F64
                | Self::Unit
                | Self::UnitStruct(_)
        )
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
            // See `EnumProbe` for why an enum is asked for as any value.
            Self::Enum(name, variants) => deserializer.deserialize_any(EnumProbe {
                inner: visitor.inner,
                name,
                variants,
            }),
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
    /// dropped and the value beneath it read with the target's own request; a local tag reaches
    /// the target as it came, and a target that refuses it is told which tag it refused.
    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Self::Value, A::Error> {
        let (tag, access) = data.variant_seed(VariantName)?;
        if is_global(&tag) {
            return access.newtype_variant_seed(Beneath {
                inner: self.inner,
                request: self.request,
                tag,
            });
        }
        // Only an enum request names a variant by a local tag, and it does not come here (see
        // `EnumProbe`), so a target that refuses an enum has refused the tag.
        let replay = Replay {
            variant: tag.clone(),
            access,
        };
        self.inner
            .visit_enum(replay)
            .map_err(|_| local_tag_refused(&tag))
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
    /// The dropped tag, as `ayaml` names it.
    tag: String,
}

impl<'de, V: Visitor<'de>> DeserializeSeed<'de> for Beneath<V> {
    type Value = V::Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<V::Value, D::Error> {
        // `make` wraps the visitor, which carries the rule to every value beneath; wrapping the
        // deserializer too would wrap the visitor again at each level.
        let request = self.request.beneath_tag();
        if !self.request.is_scalar() {
            return request.make(deserializer, self.inner);
        }
        // Beneath a tag in YAML's own namespace `ayaml` keeps a scalar as text, and only those
        // tags reach a typed target, so a scalar target refuses the text. Say why, naming the tag,
        // rather than leave the author to wonder why `true` is not a boolean.
        let expected = (&self.inner as &dyn de::Expected).to_string();
        request.make(deserializer, self.inner).map_err(|_| {
            de::Error::custom(format_args!(
                "the tag `{}` cannot be read here: aplan reads YAML's core schema, so it reads \
                 the value beneath a tag outside it as text, where {expected} is expected; remove \
                 the tag",
                written_global(&self.tag)
            ))
        })
    }
}

/// An enum request, made of `ayaml` as a request for any value.
///
/// `ayaml` names a variant by a tag (`!Circle 2.5`), by a string (`Empty`), or by the key of a
/// single-entry mapping (`{Circle: 2.5}`), and hands all three to an enum target alike. Asked for
/// an enum, then, it gives the target no way to tell a dropped global tag (`!!timestamp done`)
/// from a key or a string that is merely spelled like one (`{"<x>": done}`). Asked for any value,
/// it presents only a tag as an enum, so this probe drops a global tag, hands a local one on as
/// the variant `ayaml` would select, and reads the other two forms as `ayaml` reads them, with its
/// refusals and their places.
struct EnumProbe<V> {
    inner: V,
    name: &'static str,
    variants: &'static [&'static str],
}

const NOT_AN_ENUM: &str = "enum value must be tagged or a single-entry mapping";
const ONE_VARIANT: &str = "enum mapping must contain one variant";
const NOT_A_VARIANT_KEY: &str = "enum variant key must be a string";
const NOT_A_UNIT: &str = "expected a unit enum variant";

/// The visits of a node that is none of the kinds a visitor reads, each refused with `$refusal`.
macro_rules! refuse_other_nodes {
    ($refusal:expr) => {
        fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
            Err(E::custom($refusal))
        }

        fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
            Err(E::custom($refusal))
        }

        fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
            Err(E::custom($refusal))
        }

        fn visit_i128<E: de::Error>(self, _: i128) -> Result<Self::Value, E> {
            Err(E::custom($refusal))
        }

        fn visit_u128<E: de::Error>(self, _: u128) -> Result<Self::Value, E> {
            Err(E::custom($refusal))
        }

        fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
            Err(E::custom($refusal))
        }

        fn visit_seq<A: SeqAccess<'de>>(self, _: A) -> Result<Self::Value, A::Error> {
            Err(de::Error::custom($refusal))
        }
    };
}

impl<'de, V: Visitor<'de>> Visitor<'de> for EnumProbe<V> {
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.expecting(formatter)
    }

    refuse_other_nodes!(NOT_AN_ENUM);

    fn visit_unit<E: de::Error>(self) -> Result<V::Value, E> {
        Err(E::custom(NOT_AN_ENUM))
    }

    fn visit_str<E: de::Error>(self, variant: &str) -> Result<V::Value, E> {
        self.inner.visit_enum(Named {
            variant: variant.to_owned(),
            error: PhantomData,
        })
    }

    /// A single-entry mapping names the variant by its key. `ayaml` counts the entries before it
    /// reads the key; a reader that cannot count is checked once the entry is read.
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<V::Value, A::Error> {
        if map.size_hint().is_some_and(|entries| entries != 1) {
            return Err(de::Error::custom(ONE_VARIANT));
        }
        let read = self.inner.visit_enum(Single(&mut map))?;
        if map.next_key::<de::IgnoredAny>()?.is_some() {
            return Err(de::Error::custom(ONE_VARIANT));
        }
        Ok(read)
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<V::Value, A::Error> {
        let (tag, access) = data.variant_seed(VariantName)?;
        if is_global(&tag) {
            return access.newtype_variant_seed(Beneath {
                inner: self.inner,
                request: Request::Enum(self.name, self.variants),
                tag,
            });
        }
        self.inner.visit_enum(Replay {
            variant: local_name(&tag).to_owned(),
            access,
        })
    }
}

/// A variant named by a string, which has no value: as `ayaml` reads it, a variant that takes one
/// reads it from null.
struct Named<E> {
    variant: String,
    error: PhantomData<E>,
}

impl<'de, E: de::Error> EnumAccess<'de> for Named<E> {
    type Error = E;
    type Variant = Self;

    fn variant_seed<S: DeserializeSeed<'de>>(self, seed: S) -> Result<(S::Value, Self), E> {
        let variant = seed.deserialize(de::value::StrDeserializer::<E>::new(&self.variant))?;
        Ok((variant, self))
    }
}

impl<'de, E: de::Error> VariantAccess<'de> for Named<E> {
    type Error = E;

    fn unit_variant(self) -> Result<(), E> {
        Ok(())
    }

    fn newtype_variant_seed<S: DeserializeSeed<'de>>(self, seed: S) -> Result<S::Value, E> {
        seed.deserialize(de::value::UnitDeserializer::new())
    }

    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, E> {
        de::value::UnitDeserializer::new().deserialize_tuple(len, visitor)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, E> {
        de::value::UnitDeserializer::new().deserialize_struct("", fields, visitor)
    }
}

/// A variant named by the key of a single-entry mapping, whose value is the variant's. Both are
/// read through the mapping, so an error in either keeps its place and its path.
struct Single<'a, A>(&'a mut A);

impl<'de, A: MapAccess<'de>> EnumAccess<'de> for Single<'_, A> {
    type Error = A::Error;
    type Variant = Self;

    fn variant_seed<S: DeserializeSeed<'de>>(self, seed: S) -> Result<(S::Value, Self), A::Error> {
        match self.0.next_key_seed(VariantKey(seed))? {
            Some(variant) => Ok((variant, self)),
            None => Err(de::Error::custom(ONE_VARIANT)),
        }
    }
}

impl<'de, A: MapAccess<'de>> VariantAccess<'de> for Single<'_, A> {
    type Error = A::Error;

    fn unit_variant(self) -> Result<(), A::Error> {
        self.0.next_value_seed(UnitValue)
    }

    fn newtype_variant_seed<S: DeserializeSeed<'de>>(self, seed: S) -> Result<S::Value, A::Error> {
        self.0.next_value_seed(Within(seed))
    }

    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, A::Error> {
        self.0.next_value_seed(Made {
            request: Request::Tuple(len),
            visitor,
        })
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, A::Error> {
        self.0.next_value_seed(Made {
            request: Request::Struct("", fields),
            visitor,
        })
    }
}

/// A request made of a value with a visitor already chosen.
struct Made<V> {
    request: Request,
    visitor: V,
}

impl<'de, V: Visitor<'de>> DeserializeSeed<'de> for Made<V> {
    type Value = V::Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<V::Value, D::Error> {
        self.request.make(deserializer, self.visitor)
    }
}

/// The key that names a variant. As `ayaml` reads it, it must be a string, which the non-specific
/// tag `!` leaves it; the rule drops a global tag on it too.
struct VariantKey<S>(S);

impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for VariantKey<S> {
    type Value = S::Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<S::Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de, S: DeserializeSeed<'de>> Visitor<'de> for VariantKey<S> {
    type Value = S::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an enum variant's name")
    }

    refuse_other_nodes!(NOT_A_VARIANT_KEY);

    fn visit_unit<E: de::Error>(self) -> Result<S::Value, E> {
        Err(E::custom(NOT_A_VARIANT_KEY))
    }

    fn visit_map<A: MapAccess<'de>>(self, _: A) -> Result<S::Value, A::Error> {
        Err(de::Error::custom(NOT_A_VARIANT_KEY))
    }

    fn visit_str<E: de::Error>(self, variant: &str) -> Result<S::Value, E> {
        self.0.deserialize(de::value::StrDeserializer::new(variant))
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<S::Value, A::Error> {
        let (tag, access) = data.variant_seed(VariantName)?;
        if is_global(&tag) || local_name(&tag).is_empty() {
            return access.newtype_variant_seed(self);
        }
        Err(de::Error::custom(NOT_A_VARIANT_KEY))
    }
}

/// The value of a unit variant written as a mapping, which must be null. As `ayaml` reads it, a
/// tag outside YAML's own namespace is passed over, as it is for any target that is not an enum.
struct UnitValue;

impl<'de> DeserializeSeed<'de> for UnitValue {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for UnitValue {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("null")
    }

    refuse_other_nodes!(NOT_A_UNIT);

    fn visit_str<E: de::Error>(self, _: &str) -> Result<(), E> {
        Err(E::custom(NOT_A_UNIT))
    }

    fn visit_map<A: MapAccess<'de>>(self, _: A) -> Result<(), A::Error> {
        Err(de::Error::custom(NOT_A_UNIT))
    }

    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<(), A::Error> {
        let (tag, access) = data.variant_seed(VariantName)?;
        if in_yaml_namespace(&tag) {
            return Err(de::Error::custom(NOT_A_UNIT));
        }
        access.newtype_variant_seed(self)
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
    use super::{NOT_A_UNIT, ONE_VARIANT, Tags, from_str, from_str_strict};
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
        // Beneath any other global tag, `ayaml` reads `~` as null for a unit target, which the
        // bridge refused.
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
    fn every_request_reads_beneath_a_local_tag_as_the_bridge_did_but_a_unit() {
        // `ayaml` reads a typed value past a local tag, as the bridge did, and both refuse it
        // where the target takes any value. They part on a unit, where `ayaml` reads `~` as
        // null and the bridge refused the tag (`A2-179`).
        let mut parted = Vec::new();
        every_request("!custom ", &mut |kind, input, read| {
            if kind.starts_with("byte") {
                // The bridge reads no bytes, tagged or not.
                return;
            }
            let read = read(input);
            if read.ours.is_ok() != read.bridge.is_ok()
                || (read.ours.is_ok() && read.ours != read.bridge)
            {
                parted.push(kind.to_owned());
            }
        });
        assert_eq!(parted, ["unit", "unit struct"]);
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
            ("a: !!binary [1]\n", json!({"a": [1]})),
            (
                "a: [!!timestamp 2026-01-01, 2]\n",
                json!({"a": ["2026-01-01", 2]}),
            ),
        ] {
            assert_eq!(json(input), expected, "{input:?}");
            assert!(
                yaml_compat::compat::from_str::<serde_json::Value>(input).is_err(),
                "{input:?}: `ayaml` alone refuses it, so the rule is what reads it"
            );
        }
        // `ayaml` itself drops a core collection tag and reads a key past its tag, so these read
        // the same with the rule or without it.
        for (input, expected) in [
            ("a: !!set {x, y}\n", json!({"a": {"x": null, "y": null}})),
            ("!!timestamp 2026-01-01: x\n", json!({"2026-01-01": "x"})),
        ] {
            assert_eq!(json(input), expected, "{input:?}");
            assert_eq!(
                yaml_compat::compat::from_str::<serde_json::Value>(input).ok(),
                Some(expected),
                "{input:?}"
            );
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
        fn shown<E: fmt::Display>(read: Result<serde_json::Value, E>) -> Result<String, String> {
            read.map(|value| value.to_string())
                .map_err(|error| error.to_string())
        }
        for input in [
            "a: 1\nb: [x, 'y', \"z\"]\nc: {d: ~}\n",
            "title: Fix it! Now\n",
            "a: !!int 3\nb: !!str 4\n",
            "",
            "a: [1,\n2]\n",
            "a: 1\na: 2\n",
            "a: 1\n---\nb: 2\n",
            "a: {b\n",
        ] {
            assert_eq!(
                shown(from_str(input)),
                shown(yaml_compat::compat::from_str(input)),
                "{input:?}"
            );
            assert_eq!(
                shown(from_str_strict(input)),
                shown(yaml_compat::from_str(input)),
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

    #[test]
    fn a_tagged_document_keeps_the_strict_grammar() {
        // Each reads differently under the compatibility grammar, which once read these
        // documents (`A2-179`): `?x` in a flow sequence is a mapping there, and a byte-order mark
        // before `---` or an anchor with a colon in it is refused.
        for (input, expected) in [
            (
                "tags: [?x]\ntitle: !!timestamp 2026\n",
                json!({"tags": ["?x"], "title": "2026"}),
            ),
            (
                "\u{feff}---\ntitle: !!timestamp 2026\n",
                json!({"title": "2026"}),
            ),
            (
                "a: &x:y !!timestamp 1\nb: *x:y\n",
                json!({"a": "1", "b": "1"}),
            ),
        ] {
            let strict: serde_json::Value =
                from_str_strict(input).unwrap_or_else(|error| panic!("{input:?}: {error}"));
            assert_eq!(strict, expected, "{input:?}");
            assert_ne!(
                from_str::<serde_json::Value>(input).ok(),
                Some(expected),
                "{input:?} reads otherwise under the compatibility grammar"
            );
        }
    }

    #[derive(Deserialize, Debug, PartialEq)]
    #[serde(rename_all = "lowercase")]
    enum Status {
        Todo,
        Done,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct Tracked {
        status: Status,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    enum Figure {
        Empty,
        Circle(f64),
        Pair(u8, u8),
        Point { x: i8, y: i8 },
        Maybe(Option<u8>),
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct Drawn {
        shape: Figure,
    }

    #[test]
    fn an_enum_written_without_a_tag_reads_exactly_as_ayaml_reads_it() {
        // A variant named by a string or by a mapping's key, whatever it is spelled like, and
        // every refusal of either, message, place and path included (`A2-179`).
        for input in [
            "status: done",
            "status: {done: ~}",
            "status: {\"<x>\": done}",
            "status: <x>",
            "status: '<tag:yaml.org,2002:str>'",
            "status: {\"<tag:yaml.org,2002:str>\": done}",
            "status: bogus",
            "status: {bogus: ~}",
            "status: {}",
            "status: {done: ~, todo: ~}",
            "status: 1",
            "status: -1",
            "status: 1.5",
            "status: true",
            "status: ~",
            "status: [done]",
            "status: 99999999999999999999",
            "status: -99999999999999999999",
            "status: {1: x}",
            "status: {-1: x}",
            "status: {1.5: x}",
            "status: {~: x}",
            "status: {true: x}",
            "status: {[a]: x}",
            "status: {{a: 1}: x}",
            "status: {99999999999999999999: x}",
            "status: {-99999999999999999999: x}",
            "status: {! done: ~}",
            "status: {!local done: ~}",
            "status: {done: 5}",
            "status: {done: -1}",
            "status: {done: 1.5}",
            "status: {done: true}",
            "status: {done: x}",
            "status: {done: [1]}",
            "status: {done: {a: 1}}",
            "status: {done: 99999999999999999999}",
            "status: {done: -99999999999999999999}",
            "status: {done: !t ~}",
            "status: {done: !<tag:example.com,2000:x> ~}",
            "status: {done: !!timestamp ~}",
            "status: !done ~",
            "status: !bogus ~",
            "status: !%3Cx%3E todo",
            "status: ! todo",
        ] {
            let read = outcomes::<Tracked>(input);
            assert_eq!(read.ours, read.ayaml, "{input:?}");
        }
        for input in [
            "shape: {Square: 1}",
            "shape: { ? Square : 1 }",
            "shape: Maybe",
            "shape: {Maybe: ~}",
            "shape: {Circle: 2.5}",
            "shape: {Circle: x}",
            "shape: {Pair: [1, 2]}",
            "shape: {Pair: [1]}",
            "shape: {Point: {x: 1, y: 2}}",
            "shape: {Point: {x: 1}}",
        ] {
            let read = outcomes::<Drawn>(input);
            assert_eq!(read.ours, read.ayaml, "{input:?}");
        }
    }

    #[test]
    fn an_error_inside_a_variant_named_without_a_mapping_does_not_name_the_variant() {
        // `ayaml` adds a variant named by a string or a local tag to the path of an error inside
        // its value. Asked for any value, as the rule asks for an enum, it has no key to add, and
        // no deserializer outside it can add one, so only the path differs.
        for (input, variant) in [
            ("shape: Circle", "Circle"),
            ("shape: Pair", "Pair"),
            ("shape: Point", "Point"),
            ("shape: !Circle x", "Circle"),
            ("shape: !Point {x: 1}", "Point"),
        ] {
            let read = outcomes::<Drawn>(input);
            let ayaml = read.ayaml.expect_err("refused");
            assert_eq!(
                read.ours,
                Err(ayaml.replacen(&format!("shape.{variant}:"), "shape:", 1)),
                "{input:?}"
            );
        }
    }

    #[test]
    fn a_global_tag_over_an_enum_is_dropped_and_adds_nothing_to_the_path() {
        for (input, expected) in [
            ("status: !!timestamp done", Status::Done),
            ("status: !<tag:example.com,2000:s> todo", Status::Todo),
            ("status: {!!timestamp done: ~}", Status::Done),
            ("status: !!timestamp {done: ~}", Status::Done),
        ] {
            assert_eq!(
                from_str::<Tracked>(input).ok(),
                Some(Tracked { status: expected }),
                "{input:?}"
            );
        }
        for (input, expected) in [
            (
                "status: !!timestamp bogus",
                "status: unknown variant `bogus`, expected `todo` or `done` at line 1 column 21",
            ),
            (
                "status: !!timestamp {\"<x>\": done}",
                "status: unknown variant `<x>`, expected `todo` or `done` at line 1 column 22",
            ),
            (
                "status: {!local done: ~}",
                "status: enum variant key must be a string at line 1 column 17",
            ),
        ] {
            assert_eq!(
                from_str::<Tracked>(input).map_err(|error| error.to_string()),
                Err(expected.to_owned()),
                "{input:?}"
            );
        }
        let error = from_str::<Drawn>("shape: !!timestamp {Circle: x}")
            .expect_err("`x` is not a number")
            .to_string();
        assert!(error.starts_with("shape.Circle: invalid type"), "{error}");
    }

    #[test]
    fn a_scalar_beneath_a_tag_in_yamls_namespace_is_refused_by_naming_the_tag() {
        #[derive(Deserialize, Debug)]
        #[allow(dead_code, reason = "only the refusal is read")]
        struct Typed {
            flag: Option<bool>,
            count: Option<u32>,
            ratio: Option<f64>,
            nothing: Option<()>,
            wrapped: Option<Count>,
        }
        for (input, tag, expected) in [
            ("flag: !!timestamp true", "!!timestamp", "a boolean"),
            ("count: !!timestamp 30", "!!timestamp", "u32"),
            ("count: !!binary 30", "!!binary", "u32"),
            ("ratio: !!timestamp 1.5", "!!timestamp", "f64"),
            ("nothing: !!timestamp ~", "!!timestamp", "unit"),
            ("wrapped: !!timestamp 5", "!!timestamp", "u32"),
        ] {
            let error = from_str::<Typed>(input)
                .expect_err("the text beneath the tag is not the scalar asked for")
                .to_string();
            let field = input.split(':').next().unwrap_or_default();
            assert!(
                error.starts_with(&format!(
                    "{field}: the tag `{tag}` cannot be read here: aplan reads YAML's core \
                     schema, so it reads the value beneath a tag outside it as text, where \
                     {expected} is expected; remove the tag at line 1"
                )),
                "{input:?}: {error}"
            );
        }
    }

    #[test]
    fn a_local_tag_is_named_as_it_can_be_written_back() {
        for (input, named) in [
            ("a: ! 1\n", "the non-specific tag `!`"),
            ("a: !%3Cx%3E 1\n", "the local tag `!%3Cx%3E`"),
            ("a: !a%25b 1\n", "the local tag `!a%25b`"),
            ("a: !<!a,b> 1\n", "the local tag `!<!a,b>`"),
            ("a: !%E2%9C%93 1\n", "the local tag `!%E2%9C%93`"),
        ] {
            let error = from_str::<serde_json::Value>(input)
                .expect_err("a local tag cannot be read as JSON")
                .to_string();
            assert!(
                error.starts_with(&format!("a: {named} cannot be read here")),
                "{input:?}: {error}"
            );
        }
    }

    /// A mapping whose size is not known until it has been read, which `ayaml`'s is.
    struct Uncounted(
        std::vec::IntoIter<(&'static str, Option<&'static str>)>,
        Option<&'static str>,
    );

    impl<'de> de::MapAccess<'de> for Uncounted {
        type Error = de::value::Error;

        fn next_key_seed<K: de::DeserializeSeed<'de>>(
            &mut self,
            seed: K,
        ) -> Result<Option<K::Value>, Self::Error> {
            let Some((key, value)) = self.0.next() else {
                return Ok(None);
            };
            self.1 = value;
            seed.deserialize(de::value::StrDeserializer::new(key))
                .map(Some)
        }

        fn next_value_seed<V: de::DeserializeSeed<'de>>(
            &mut self,
            seed: V,
        ) -> Result<V::Value, Self::Error> {
            match self.1.take() {
                Some(value) => seed.deserialize(de::value::StrDeserializer::new(value)),
                None => seed.deserialize(de::value::UnitDeserializer::new()),
            }
        }
    }

    #[test]
    fn a_mapping_of_unknown_size_still_names_exactly_one_variant() {
        let read = |entries: Vec<(&'static str, Option<&'static str>)>| {
            Status::deserialize(Tags(de::value::MapAccessDeserializer::new(Uncounted(
                entries.into_iter(),
                None,
            ))))
            .map_err(|error| error.to_string())
        };
        assert_eq!(read(vec![("done", None)]), Ok(Status::Done));
        assert_eq!(read(vec![("done", Some("x"))]), Err(NOT_A_UNIT.to_owned()));
        assert_eq!(read(vec![]), Err(ONE_VARIANT.to_owned()));
        assert_eq!(
            read(vec![("done", None), ("todo", None)]),
            Err(ONE_VARIANT.to_owned())
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

    /// A visitor that takes the name of the variant it is given.
    struct VariantEcho;

    impl<'de> Visitor<'de> for VariantEcho {
        type Value = String;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a variant")
        }

        fn visit_enum<A: de::EnumAccess<'de>>(self, data: A) -> Result<String, A::Error> {
            let (name, variant) = data.variant::<String>()?;
            de::VariantAccess::unit_variant(variant).map(|()| name)
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
        // An enum is the one request made as another, for any value (see `EnumProbe`), and the
        // string it is answered with names the variant.
        assert_eq!(
            Tags(Recorder)
                .deserialize_enum("E", &["V"], VariantEcho)
                .expect("the recorder answers"),
            "deserialize_any()"
        );
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
