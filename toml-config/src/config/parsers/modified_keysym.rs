use {
    crate::{
        config::{
            keysyms::KEYSYMS,
            parser::{DataType, ParseResult, Parser, UnexpectedDataType},
        },
        toml::toml_span::{Span, SpannedExt},
    },
    jay_config::keyboard::{
        mods::{Modifiers, ALT, CAPS, CTRL, LOCK, LOGO, MOD1, MOD2, MOD3, MOD4, MOD5, NUM, SHIFT},
        ModifiedKeySym,
    },
    thiserror::Error,
};

#[derive(Debug, Error)]
pub enum ModifiedKeysymParserError {
    #[error(transparent)]
    Expected(#[from] UnexpectedDataType),
    #[error("You cannot use more than one non-modifier key")]
    MoreThanOneSym,
    #[error("You must specify exactly one non-modifier key")]
    MissingSym,
    #[error("Unknown keysym {0}")]
    UnknownKeysym(String),
}

pub struct ModifiedKeysymParser;

impl Parser for ModifiedKeysymParser {
    type Value = ModifiedKeySym;
    type Error = ModifiedKeysymParserError;
    const EXPECTED: &'static [DataType] = &[DataType::String];

    fn parse_string(&mut self, span: Span, string: &str) -> ParseResult<Self> {
        let mut modifiers = Modifiers(0);
        let mut sym = None;
        for part in string.split("-") {
            let modifier = match part {
                "shift" => SHIFT,
                "lock" => LOCK,
                "ctrl" => CTRL,
                "mod1" => MOD1,
                "mod2" => MOD2,
                "mod3" => MOD3,
                "mod4" => MOD4,
                "mod5" => MOD5,
                "caps" => CAPS,
                "alt" => ALT,
                "num" => NUM,
                "logo" => LOGO,
                _ => match KEYSYMS.get(part) {
                    Some(new) if sym.is_none() => {
                        sym = Some(*new);
                        continue;
                    }
                    Some(_) => return Err(ModifiedKeysymParserError::MoreThanOneSym.spanned(span)),
                    _ => {
                        return Err(ModifiedKeysymParserError::UnknownKeysym(part.to_string())
                            .spanned(span))
                    }
                },
            };
            modifiers |= modifier;
        }
        match sym {
            Some(s) => Ok(modifiers | s),
            None => Err(ModifiedKeysymParserError::MissingSym.spanned(span)),
        }
    }
}
