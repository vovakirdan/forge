//! Explicit local master-key setup, independent of database/service startup.

use clap::{Parser, ValueEnum};
use forge_provider_common::{MasterKeySource, SecretStore};
use std::{path::PathBuf, process::ExitCode};

#[derive(Parser)]
struct Arguments {
    #[arg(long)]
    directory: PathBuf,
    /// Required explicit selection; unavailable keyring never silently falls back.
    #[arg(long, value_enum)]
    initialize: Option<Source>,
}

#[derive(Clone, Copy, ValueEnum)]
enum Source {
    Keyring,
    File,
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
    let result = match arguments.initialize {
        Some(source) => SecretStore::initialize(
            &arguments.directory,
            match source {
                Source::Keyring => MasterKeySource::LinuxKeyring,
                Source::File => MasterKeySource::OwnerOnlyFile,
            },
        ),
        None => SecretStore::load(&arguments.directory),
    };
    match result {
        Ok(store) => {
            println!(
                "Forge Secret Store ready: key_id={}",
                store.binding().key_id
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("forge-secret-store: {error}");
            ExitCode::FAILURE
        }
    }
}
