//! PostgreSQL dump inspection before any import client is invoked.

mod reader;

pub use reader::{
    DumpSummary, ImportError, RegularDumpFormat, detect_regular_dump_format, inspect_plain_dump,
};
