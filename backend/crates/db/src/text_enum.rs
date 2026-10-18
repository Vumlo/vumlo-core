/// Defines `as_str`, `FromStr` and the sqlx impls for a fieldless enum stored as `text`.
/// Stored as `text` + CHECK (AGENTS §8), not as a Postgres ENUM. The crate that uses it needs
/// `sqlx`.
#[macro_export]
macro_rules! text_enum {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, String> {
                match s {
                    $($text => Ok(Self::$variant),)+
                    other => Err(format!(
                        "unknown {} `{other}` (expected one of: {})",
                        stringify!($name).to_lowercase(),
                        [$($text),+].join(", ")
                    )),
                }
            }
        }

        impl ::sqlx::Type<::sqlx::Postgres> for $name {
            fn type_info() -> ::sqlx::postgres::PgTypeInfo {
                <str as ::sqlx::Type<::sqlx::Postgres>>::type_info()
            }
            fn compatible(ty: &::sqlx::postgres::PgTypeInfo) -> bool {
                <str as ::sqlx::Type<::sqlx::Postgres>>::compatible(ty)
            }
        }

        impl<'r> ::sqlx::Decode<'r, ::sqlx::Postgres> for $name {
            fn decode(
                value: ::sqlx::postgres::PgValueRef<'r>,
            ) -> Result<Self, ::sqlx::error::BoxDynError> {
                let text = <&str as ::sqlx::Decode<::sqlx::Postgres>>::decode(value)?;
                text.parse().map_err(Into::into)
            }
        }
    };
}
