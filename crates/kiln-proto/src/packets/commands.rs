//! Command tree, command execution and tab-completion packets (play state).
//! Layouts follow the 26.3 bytecode of `ClientboundCommandsPacket` (including `FLAG_RESTRICTED`),
//! `ClientboundCommandSuggestionsPacket`, `ServerboundChatCommand[Signed]Packet` and
//! `ServerboundCommandSuggestionPacket`.

use super::packet;
use crate::nbt::Tag;
use crate::{DecodeError, Reader, WriteExt};
use bytes::{BufMut, Bytes};
use kiln_data::packets as ids;

const TYPE_ROOT: u8 = 0;
const TYPE_LITERAL: u8 = 1;
const TYPE_ARGUMENT: u8 = 2;
const FLAG_EXECUTABLE: u8 = 0x04;
const FLAG_REDIRECT: u8 = 0x08;
const FLAG_CUSTOM_SUGGESTIONS: u8 = 0x10;
const FLAG_RESTRICTED: u8 = 0x20;

/// One entry of the flattened command graph; `children` and `redirect` index into the node list.
#[derive(Debug, Clone, PartialEq)]
pub struct Node<'a> {
    pub kind: NodeKind<'a>,
    pub executable: bool,
    /// The node's requirement fails for a source without permissions.
    pub restricted: bool,
    pub children: Vec<i32>,
    pub redirect: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind<'a> {
    Root,
    Literal(&'a str),
    /// `suggestions` is a suggestion provider id such as `minecraft:ask_server`.
    Argument {
        name: &'a str,
        parser: Parser<'a>,
        suggestions: Option<&'a str>,
    },
}

/// Brigadier `StringArgumentType.StringType`, written as its ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringKind {
    Word = 0,
    Phrase = 1,
    Greedy = 2,
}

/// An argument parser with the properties its `ArgumentTypeInfo` serializes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Parser<'a> {
    Float {
        min: Option<f32>,
        max: Option<f32>,
    },
    Double {
        min: Option<f64>,
        max: Option<f64>,
    },
    Integer {
        min: Option<i32>,
        max: Option<i32>,
    },
    Long {
        min: Option<i64>,
        max: Option<i64>,
    },
    String(StringKind),
    Entity {
        single: bool,
        players_only: bool,
    },
    ScoreHolder {
        multiple: bool,
    },
    Time {
        min: i32,
    },
    /// A parser whose only property is a registry key: `minecraft:resource`, `resource_key`,
    /// `resource_or_tag`, `resource_or_tag_key` or `resource_selector`.
    Registry {
        id: &'a str,
        registry: &'a str,
    },
    /// A parser without properties, e.g. `brigadier:bool` or `minecraft:vec3`.
    Plain(&'a str),
}

const WITH_PROPERTIES: &[&str] = &[
    "brigadier:float",
    "brigadier:double",
    "brigadier:integer",
    "brigadier:long",
    "brigadier:string",
    "minecraft:entity",
    "minecraft:score_holder",
    "minecraft:time",
    "minecraft:resource",
    "minecraft:resource_key",
    "minecraft:resource_or_tag",
    "minecraft:resource_or_tag_key",
    "minecraft:resource_selector",
];

impl Parser<'_> {
    pub fn id(&self) -> &str {
        match self {
            Parser::Float { .. } => "brigadier:float",
            Parser::Double { .. } => "brigadier:double",
            Parser::Integer { .. } => "brigadier:integer",
            Parser::Long { .. } => "brigadier:long",
            Parser::String(_) => "brigadier:string",
            Parser::Entity { .. } => "minecraft:entity",
            Parser::ScoreHolder { .. } => "minecraft:score_holder",
            Parser::Time { .. } => "minecraft:time",
            Parser::Registry { id, .. } => id,
            Parser::Plain(id) => id,
        }
    }

    fn write(&self, b: &mut bytes::BytesMut) {
        let id = self.id();
        let network_id = kiln_data::builtin_id("minecraft:command_argument_type", id)
            .unwrap_or_else(|| panic!("unknown command argument type {id}"));
        b.put_varint(network_id);
        let flags = |min: bool, max: bool| (min as u8) | (max as u8) << 1;
        match *self {
            Parser::Float { min, max } => {
                b.put_u8(flags(min.is_some(), max.is_some()));
                min.into_iter().chain(max).for_each(|v| b.put_f32(v));
            }
            Parser::Double { min, max } => {
                b.put_u8(flags(min.is_some(), max.is_some()));
                min.into_iter().chain(max).for_each(|v| b.put_f64(v));
            }
            Parser::Integer { min, max } => {
                b.put_u8(flags(min.is_some(), max.is_some()));
                min.into_iter().chain(max).for_each(|v| b.put_i32(v));
            }
            Parser::Long { min, max } => {
                b.put_u8(flags(min.is_some(), max.is_some()));
                min.into_iter().chain(max).for_each(|v| b.put_i64(v));
            }
            Parser::String(kind) => b.put_varint(kind as i32),
            Parser::Entity { single, players_only } => b.put_u8(single as u8 | (players_only as u8) << 1),
            Parser::ScoreHolder { multiple } => b.put_u8(multiple as u8),
            Parser::Time { min } => b.put_i32(min),
            Parser::Registry { id, registry } => {
                debug_assert!(WITH_PROPERTIES[8..].contains(&id), "{id} has no registry property");
                b.put_string(registry);
            }
            Parser::Plain(id) => debug_assert!(!WITH_PROPERTIES.contains(&id), "{id} needs properties"),
        }
    }
}

/// `commands`: the command graph the client uses for parsing, highlighting and completion.
pub fn commands(nodes: &[Node], root: i32) -> Bytes {
    let mut b = packet(ids::play::clientbound::COMMANDS);
    b.put_varint(nodes.len() as i32);
    for node in nodes {
        let mut flags = match node.kind {
            NodeKind::Root => TYPE_ROOT,
            NodeKind::Literal(_) => TYPE_LITERAL,
            NodeKind::Argument { .. } => TYPE_ARGUMENT,
        };
        if node.executable {
            flags |= FLAG_EXECUTABLE;
        }
        if node.redirect.is_some() {
            flags |= FLAG_REDIRECT;
        }
        if matches!(node.kind, NodeKind::Argument { suggestions: Some(_), .. }) {
            flags |= FLAG_CUSTOM_SUGGESTIONS;
        }
        if node.restricted {
            flags |= FLAG_RESTRICTED;
        }
        b.put_u8(flags);
        b.put_varint(node.children.len() as i32);
        for &c in &node.children {
            b.put_varint(c);
        }
        if let Some(r) = node.redirect {
            b.put_varint(r);
        }
        match &node.kind {
            NodeKind::Root => {}
            NodeKind::Literal(name) => b.put_string(name),
            NodeKind::Argument { name, parser, suggestions } => {
                b.put_string(name);
                parser.write(&mut b);
                if let Some(s) = suggestions {
                    b.put_string(s);
                }
            }
        }
    }
    b.put_varint(root);
    b.freeze()
}

/// A completion; `tooltip` is a chat component.
#[derive(Debug, Clone, Copy)]
pub struct SuggestionEntry<'a> {
    pub text: &'a str,
    pub tooltip: Option<&'a Tag>,
}

/// `command_suggestions`: the answer to a `command_suggestion` request. `start` and `length`
/// are in UTF-16 code units of the request text (including its leading `/`).
pub fn command_suggestions(id: i32, start: i32, length: i32, entries: &[SuggestionEntry]) -> Bytes {
    let mut b = packet(ids::play::clientbound::COMMAND_SUGGESTIONS);
    b.put_varint(id);
    b.put_varint(start);
    b.put_varint(length);
    b.put_varint(entries.len() as i32);
    for e in entries {
        b.put_string(e.text);
        b.put_bool(e.tooltip.is_some());
        if let Some(t) = e.tooltip {
            t.write_network(&mut b);
        }
    }
    b.freeze()
}

/// `chat_command`: an unsigned command, without the leading `/`.
pub fn decode_chat_command(r: &mut Reader) -> Result<String, DecodeError> {
    let command = r.string(32767)?.to_owned();
    r.finish()?;
    Ok(command)
}

/// `chat_command_signed`: sent instead of `chat_command` when the command has signable
/// arguments (e.g. `/say`, `/msg`), even when the client has no chat session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedChatCommand {
    pub command: String,
    /// Milliseconds since the Unix epoch.
    pub timestamp: i64,
    pub salt: i64,
    pub signatures: Vec<ArgumentSignature>,
    pub last_seen: LastSeenUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgumentSignature {
    pub argument: String,
    pub signature: Box<[u8; 256]>,
}

/// `LastSeenMessages.Update`: offset, a fixed 20-bit acknowledgement set, checksum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastSeenUpdate {
    pub offset: i32,
    pub acknowledged: [u8; 3],
    pub checksum: u8,
}

pub fn decode_chat_command_signed(r: &mut Reader) -> Result<SignedChatCommand, DecodeError> {
    let command = r.string(32767)?.to_owned();
    let timestamp = r.i64()?;
    let salt = r.i64()?;
    let n = r.len()?;
    if n > 8 {
        return Err(DecodeError::Invalid("more than 8 argument signatures"));
    }
    let mut signatures = Vec::with_capacity(n);
    for _ in 0..n {
        let argument = r.string(16)?.to_owned();
        let signature = Box::new(r.bytes(256)?.try_into().unwrap());
        signatures.push(ArgumentSignature { argument, signature });
    }
    let offset = r.varint()?;
    let acknowledged = r.bytes(3)?.try_into().unwrap();
    let checksum = r.u8()?;
    r.finish()?;
    Ok(SignedChatCommand {
        command,
        timestamp,
        salt,
        signatures,
        last_seen: LastSeenUpdate { offset, acknowledged, checksum },
    })
}

/// `command_suggestion`: a tab-completion request for text up to the cursor (with its `/`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionRequest {
    pub id: i32,
    pub command: String,
}

pub fn decode_command_suggestion(r: &mut Reader) -> Result<SuggestionRequest, DecodeError> {
    let id = r.varint()?;
    let command = r.string(32500)?.to_owned();
    r.finish()?;
    Ok(SuggestionRequest { id, command })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(p: &Bytes, id: i32) -> &[u8] {
        let mut r = Reader::new(p);
        assert_eq!(r.varint().unwrap(), id);
        r.rest()
    }

    #[test]
    fn node_layout() {
        let nodes = [
            Node { kind: NodeKind::Root, executable: false, restricted: false, children: vec![1], redirect: None },
            Node {
                kind: NodeKind::Literal("tp"),
                executable: true,
                restricted: true,
                children: vec![2],
                redirect: None,
            },
            Node {
                kind: NodeKind::Argument {
                    name: "n",
                    parser: Parser::Integer { min: Some(1), max: None },
                    suggestions: Some("minecraft:ask_server"),
                },
                executable: false,
                restricted: false,
                children: vec![],
                redirect: Some(0),
            },
        ];
        let p = commands(&nodes, 0);
        let int_id = kiln_data::builtin_id("minecraft:command_argument_type", "brigadier:integer").unwrap() as u8;
        let mut expected = vec![3u8];
        expected.extend([0x00, 1, 1]);
        expected.extend([0x01 | 0x04 | 0x20, 1, 2, 2, b't', b'p']);
        expected.extend([0x02 | 0x08 | 0x10, 0, 0, 1, b'n', int_id, 0x01, 0, 0, 0, 1]);
        expected.push(20);
        expected.extend(b"minecraft:ask_server");
        expected.push(0);
        assert_eq!(body(&p, ids::play::clientbound::COMMANDS), &expected[..]);
    }

    #[test]
    fn parser_properties() {
        let enc = |p: Parser| {
            let mut b = bytes::BytesMut::new();
            p.write(&mut b);
            let mut r = Reader::new(&b);
            r.varint().unwrap();
            r.rest().to_vec()
        };
        assert_eq!(enc(Parser::Plain("minecraft:vec3")), Vec::<u8>::new());
        assert_eq!(enc(Parser::Float { min: None, max: None }), vec![0]);
        assert_eq!(enc(Parser::Double { min: None, max: Some(1.0) }), [&[2u8][..], &1.0f64.to_be_bytes()].concat());
        assert_eq!(
            enc(Parser::Long { min: Some(-1), max: Some(1) }),
            [&[3u8][..], &(-1i64).to_be_bytes(), &1i64.to_be_bytes()].concat()
        );
        assert_eq!(enc(Parser::String(StringKind::Greedy)), vec![2]);
        assert_eq!(enc(Parser::Entity { single: true, players_only: true }), vec![3]);
        assert_eq!(enc(Parser::Entity { single: false, players_only: true }), vec![2]);
        assert_eq!(enc(Parser::ScoreHolder { multiple: true }), vec![1]);
        assert_eq!(enc(Parser::Time { min: -5 }), (-5i32).to_be_bytes().to_vec());
        assert_eq!(
            enc(Parser::Registry { id: "minecraft:resource", registry: "minecraft:timeline" }),
            [&[18u8][..], b"minecraft:timeline"].concat()
        );
    }

    #[test]
    fn suggestions_layout() {
        let tip = crate::nbt::text("hi");
        let p = command_suggestions(
            7,
            4,
            2,
            &[SuggestionEntry { text: "@a", tooltip: Some(&tip) }, SuggestionEntry { text: "@e", tooltip: None }],
        );
        let expected: &[u8] = &[7, 4, 2, 2, 2, b'@', b'a', 1, 8, 0, 2, b'h', b'i', 2, b'@', b'e', 0];
        assert_eq!(body(&p, ids::play::clientbound::COMMAND_SUGGESTIONS), expected);
    }

    // Vectors encoded with vanilla's own STREAM_CODECs (crates/kiln-command/tools/vanilla_check.py encode).

    #[test]
    fn chat_command_vanilla_vector() {
        let mut bytes = hex("1167616d656d6f6465206372656174697665");
        assert_eq!(decode_chat_command(&mut Reader::new(&bytes)).unwrap(), "gamemode creative");
        bytes.push(0x9f);
        assert_eq!(decode_chat_command(&mut Reader::new(&bytes)), Err(DecodeError::TrailingBytes(1)));
    }

    #[test]
    fn signed_chat_command_vanilla_vector() {
        let mut bytes =
            hex(concat!("0c736179206869207468657265", "0000019a2b3c4d5e", "fffffffffffffff9", "01076d657373616765"));
        bytes.extend(0..=255u8);
        bytes.extend(hex("0305a00c2a"));
        let cmd = decode_chat_command_signed(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(cmd.command, "say hi there");
        assert_eq!(cmd.timestamp, 0x19a2b3c4d5e);
        assert_eq!(cmd.salt, -7);
        assert_eq!(cmd.signatures.len(), 1);
        assert_eq!(cmd.signatures[0].argument, "message");
        assert_eq!(cmd.signatures[0].signature[255], 255);
        assert_eq!(cmd.last_seen, LastSeenUpdate { offset: 3, acknowledged: [0x05, 0xa0, 0x0c], checksum: 0x2a });
    }

    #[test]
    fn signed_chat_command_unsigned_client() {
        // What a client without a chat session sends: no signatures, empty acknowledgements.
        let bytes = hex("07736179206865790000000000000000000000000000002a000000000000");
        let cmd = decode_chat_command_signed(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(cmd.command, "say hey");
        assert!(cmd.signatures.is_empty());
        assert_eq!(cmd.last_seen, LastSeenUpdate { offset: 0, acknowledged: [0; 3], checksum: 0 });
    }

    #[test]
    fn command_suggestion_vanilla_vector() {
        let bytes = hex("80010b2f74656c65706f72742040");
        let req = decode_command_suggestion(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(req, SuggestionRequest { id: 128, command: "/teleport @".into() });
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }
}
