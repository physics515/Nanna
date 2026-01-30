//! Built-in tools

mod audio;
mod authoring;
mod browser;
mod curiosity;
mod echo;
mod exec;
mod file;
mod memory;
mod memory_storage;
mod schedule;
mod vision;
mod web;

#[cfg(feature = "browser")]
mod browser_wiring;

#[cfg(feature = "vision")]
mod vision_wiring;

mod audio_wiring;

pub use audio::{OpenAiTts, OpenAiWhisper, TextToSpeechTool, TranscribeFn, TranscribeTool, TtsFn};
pub use authoring::{CreateToolTool, DeleteToolTool, ListToolsTool, ScriptTool, ScriptToolExecutor, ToolStore};
pub use browser::{BrowserActionTool, BrowserEvaluateTool, BrowserExtractTool, BrowserFn, BrowserScreenshotTool};
pub use curiosity::{ExploreTool, WonderTool, StatusTool};
pub use echo::EchoTool;
pub use exec::ExecTool;
pub use file::{ReadFileTool, WriteFileTool, ListDirTool};
pub use memory::{InMemoryStorage, MemoryStorage, MemoryResult, StorageHandle, RememberTool, RecallTool, ReflectTool};
pub use memory_storage::{EmbedFn, TursoMemoryStorage};
pub use schedule::{ReminderStore, SchedulerState, RemindTool, ListRemindersTool, CancelReminderTool};
pub use vision::{AnalyzeImageTool, ScreenshotTool, VisionFn};
pub use web::{WebSearchTool, WebSearchBatchTool, WebFetchTool};

#[cfg(feature = "browser")]
pub use browser_wiring::{BrowserManager, create_browser_tools};

#[cfg(feature = "vision")]
pub use vision_wiring::create_vision_tool;

pub use audio_wiring::{create_tts_tool, create_tts_tool_with_dir, create_transcribe_tool, create_audio_tools};
