use std::fmt::Debug;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub enum ServerEvent {
    // set Hello
    HelloStart,
    HelloChunk { data: Vec<u8> },
    HelloEnd,

    ASR { text: String },
    Action { action: String },
    Choices { message: String, items: Vec<String> },
    HasNotification,
    StartAudio { text: String },
    AudioChunk { data: Vec<u8> },
    AudioChunkWithVowel { data: Vec<u8>, vowel: u8 },
    AudioChunki16 { data: Vec<i16>, vowel: u8 },
    DisplayText { text: String },
    EndAudio,
    StartVideo,
    EndVideo,
    EndResponse,

    EndVad,
}

impl Debug for ServerEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerEvent::HelloStart => write!(f, "ServerEvent::HelloStart"),
            ServerEvent::HelloChunk { data } => {
                write!(
                    f,
                    "ServerEvent::HelloChunk {{ data: [{} bytes] }}",
                    data.len()
                )
            }
            ServerEvent::HelloEnd => write!(f, "ServerEvent::HelloEnd"),
            ServerEvent::ASR { text } => write!(f, "ServerEvent::ASR {{ text: {} }}", text),
            ServerEvent::Action { action } => {
                write!(f, "ServerEvent::Action {{ action: {} }}", action)
            }
            ServerEvent::Choices { message, items } => write!(
                f,
                "ServerEvent::Choices {{ message: {}, items: {:?} }}",
                message, items
            ),
            ServerEvent::HasNotification => write!(f, "ServerEvent::HasNotification"),
            ServerEvent::StartAudio { text } => {
                write!(f, "ServerEvent::StartAudio {{ text: {} }}", text)
            }
            ServerEvent::AudioChunk { data } => {
                write!(
                    f,
                    "ServerEvent::AudioChunk {{ data: [{} bytes] }}",
                    data.len()
                )
            }
            ServerEvent::AudioChunkWithVowel { data, vowel } => write!(
                f,
                "ServerEvent::AudioChunkWithVowel {{ data: [{} bytes], vowel: {} }}",
                data.len(),
                vowel
            ),
            ServerEvent::AudioChunki16 { data, vowel } => write!(
                f,
                "ServerEvent::AudioChunki16 {{ data: [{} samples], vowel: {} }}",
                data.len(),
                vowel
            ),
            ServerEvent::DisplayText { text } => {
                write!(f, "ServerEvent::DisplayText {{ text: {} }}", text)
            }
            ServerEvent::EndAudio => write!(f, "ServerEvent::EndAudio"),
            ServerEvent::StartVideo => write!(f, "ServerEvent::StartVideo"),
            ServerEvent::EndVideo => write!(f, "ServerEvent::EndVideo"),
            ServerEvent::EndResponse => write!(f, "ServerEvent::EndResponse"),
            ServerEvent::EndVad => write!(f, "ServerEvent::EndVad"),
        }
    }
}

#[test]
fn test_rmp_command() {
    let event = ServerEvent::Action {
        action: "say".to_string(),
    };
    let data = rmp_serde::to_vec(&event).unwrap();
    println!("Serialized data: {:?}", data);
    println!("Serialized data: {}", String::from_utf8_lossy(&data));
    let data = rmp_serde::to_vec_named(&event).unwrap();
    println!("Serialized data: {:?}", data);
    println!("Serialized data: {}", String::from_utf8_lossy(&data));
    let cmd: ServerEvent = rmp_serde::from_slice(&data).unwrap();
    match cmd {
        ServerEvent::Action { action } => {
            assert_eq!(action, "say");
        }
        _ => panic!("Unexpected command: {:?}", cmd),
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "event")]
pub enum ClientCommand {
    StartRecord,
    StartChat,
    Submit,
    Text { input: String },
    Select { index: usize },
}

#[test]
fn test_rmp_client_command() {
    let cmd = ClientCommand::Text {
        input: "Hello".to_string(),
    };
    let data = serde_json::to_string(&cmd).unwrap();
    println!("Serialized data: {}", data);
    let cmd2: ClientCommand = serde_json::from_str(&data).unwrap();
    match cmd2 {
        ClientCommand::Text { input } => {
            assert_eq!(input, "Hello");
        }
        _ => panic!("Unexpected command: {:?}", cmd2),
    }
}
