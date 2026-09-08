//! Single-process input pump. Turns share one stdout/wall budget and one session.
use super::{OutputBudget, create_parent, mark_incomplete, read_private, write_private};
use forge_domain::communication::EmployeeMessage;
use forge_protocol::{
    runtime::RuntimeInputConfig,
    supervisor::v1::{
        DeliverRuntimeInput, RuntimeInputStatus as Status, deliver_runtime_input::Action,
    },
};
use forge_provider_claude::{ClaudeTurnEnd, encode_user_message, parse_jsonl_event};
use forge_provider_common::SecretBytes;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::mpsc,
};
use uuid::Uuid;

pub(super) struct Frame {
    session: Option<Uuid>,
    id: Option<Uuid>,
    echoed: Option<Uuid>,
    end: Option<ClaudeTurnEnd>,
    digest: [u8; 32],
}

pub(super) async fn forward(line: &[u8], sender: &mpsc::Sender<Frame>) -> io::Result<()> {
    let text = std::str::from_utf8(line).map_err(|_| invalid())?;
    let event = parse_jsonl_event(text).map_err(|_| invalid())?;
    // The pump closes normally after typed CloseAfterTurn. Tail output still
    // belongs in evidence; pump failures independently signal the shared budget.
    let _ = sender
        .send(Frame {
            session: event.session_id,
            id: event.event_id,
            echoed: event.replayed_message_id,
            end: event.turn_end,
            digest: Sha256::digest(line).into(),
        })
        .await;
    Ok(())
}

pub(super) struct Paths {
    pub initial: PathBuf,
    pub mailbox: PathBuf,
    pub private: PathBuf,
    pub receipts: PathBuf,
}

pub(super) async fn pump(
    mut stdin: impl AsyncWrite + Unpin,
    config: RuntimeInputConfig,
    paths: Paths,
    mut frames: mpsc::Receiver<Frame>,
    budget: Arc<OutputBudget>,
) -> io::Result<()> {
    let result = pump_inner(&mut stdin, &config, &paths, &mut frames).await;
    if result.is_err() {
        mark_incomplete(&budget);
    }
    result
}

async fn pump_inner(
    stdin: &mut (impl AsyncWrite + Unpin),
    config: &RuntimeInputConfig,
    paths: &Paths,
    frames: &mut mpsc::Receiver<Frame>,
) -> io::Result<()> {
    let session = Uuid::parse_str(&config.run_id).map_err(|_| invalid())?;
    // A new driver process must not infer that a prior write reached the model.
    // Existing marker fails closed rather than refeeding an uncertain transcript.
    write_private(&paths.private.join("native-input-started"), b"1\n")?;
    create_parent(&paths.receipts.join("placeholder"))?;
    let initial = read_private(&paths.initial)?;
    stdin.write_all(&initial).await?;
    stdin.flush().await?;
    let mut active = ActiveInput {
        id: session,
        command: None,
        ended: false,
        echoed: false,
    };
    let mut idle = false;
    let mut seen = HashMap::new();
    let mut sent = BTreeMap::<String, [u8; 32]>::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            frame=frames.recv()=>{
                let Some(frame)=frame else {return Err(invalid());};
                if frame.session.is_some_and(|id|id!=session) {return Err(invalid());}
                if let Some(id)=frame.id {
                    if let Some(prior)=seen.get(&id) {
                        if *prior!=frame.digest {return Err(invalid());}
                        continue;
                    }
                    if seen.len()>=16384 {return Err(invalid());}
                    seen.insert(id,frame.digest);
                }
                if let Some(echoed)=frame.echoed {
                    if idle || echoed!=active.id {return Err(invalid());}
                    active.echoed=true;
                    if let Some(command)=&active.command {persist_receipt(paths,command,Status::RuntimeAccepted)?;}
                }
                if let Some(end)=frame.end {
                    if idle || active.ended || end!=ClaudeTurnEnd::Completed {return Err(invalid());}
                    active.ended=true;
                }
                idle=active.ended && active.echoed;
            }
            _=tick.tick(), if idle=>{
                let commands=read_mailbox(&paths.mailbox,config)?;
                // A close request is transport control, never another model turn.
                if let Some(close)=commands.iter().find(|input|matches!(input.action,Some(Action::CloseAfterTurn(_)))) {
                    stdin.shutdown().await?;
                    persist_receipt(paths,close,Status::InputClosed)?;
                    for input in commands.iter().filter(|input|matches!(input.action,Some(Action::Message(_))) && !sent.contains_key(&input.command_id)) {
                        persist_receipt(paths,input,Status::DeliveryUnknown)?;
                    }
                    return Ok(());
                }
                for input in commands {
                    let hash:[u8;32]=Sha256::digest(serde_json::to_vec(&input).map_err(|_|invalid())?).into();
                    if let Some(prior)=sent.get(&input.command_id) {
                        if *prior!=hash {return Err(invalid());}
                        continue;
                    }
                    let Some(Action::Message(message))=&input.action else {return Err(invalid());};
                    let source:EmployeeMessage=serde_json::from_str(&message.source_message_json).map_err(|_|invalid())?;
                    // A fixed prefix prevents source text becoming a CLI slash command.
                    let prompt=SecretBytes::new(format!("Forge addressed instruction {}. Read this message, then acknowledge or reply using the Forge Inbox tools.\n{}",source.data().id,message.source_message_json).into_bytes());
                    let id=Uuid::parse_str(&input.command_id).map_err(|_|invalid())?;
                    let bytes=encode_user_message(session,id,&prompt).map_err(|_|invalid())?;
                    stdin.write_all(bytes.expose()).await?;stdin.flush().await?;
                    sent.insert(input.command_id.clone(),hash);
                    active=ActiveInput{id,command:Some(input),ended:false,echoed:false};
                    idle=false;
                    break;
                }
            }
        }
    }
}

struct ActiveInput {
    id: Uuid,
    command: Option<DeliverRuntimeInput>,
    ended: bool,
    echoed: bool,
}

fn read_mailbox(path: &Path, config: &RuntimeInputConfig) -> io::Result<Vec<DeliverRuntimeInput>> {
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut inputs = Vec::new();
    for entry in entries.take(130) {
        if inputs.len() == 129 {
            return Err(invalid());
        }
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(".pending-") {
            continue;
        }
        let bytes = match read_private(&entry.path()) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if bytes.len() > crate::runtime_input::MAX_INPUT_BYTES as usize {
            return Err(invalid());
        }
        let input: DeliverRuntimeInput = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        let expected = format!("{:020}-{}.json", input.sequence, input.command_id);
        if entry.file_name() != std::ffi::OsStr::new(&expected)
            || input.run_id != config.run_id
            || input.lease_fencing_token != config.fencing_token
            || input.environment_epoch != config.environment_epoch
            || input.sequence == 0
            || Uuid::parse_str(&input.command_id).is_err()
        {
            return Err(invalid());
        }
        inputs.push(input);
    }
    inputs.sort_by_key(|input| input.sequence);
    if inputs
        .windows(2)
        .any(|pair| pair[0].sequence == pair[1].sequence)
    {
        return Err(invalid());
    }
    Ok(inputs)
}

fn persist_receipt(paths: &Paths, input: &DeliverRuntimeInput, status: Status) -> io::Result<()> {
    let receipt = crate::runtime_input::receipt(input, status);
    let path = paths.receipts.join(format!("{}.json", input.command_id));
    if path.try_exists()? {
        let prior: forge_protocol::supervisor::v1::RuntimeInputReceipt =
            serde_json::from_slice(&read_private(&path)?).map_err(|_| invalid())?;
        if prior.input_command_id == receipt.input_command_id
            && prior.status == receipt.status
            && prior.run_id == receipt.run_id
            && prior.lease_fencing_token == receipt.lease_fencing_token
            && prior.environment_epoch == receipt.environment_epoch
        {
            return Ok(());
        }
        return Err(invalid());
    }
    crate::runtime_input::immutable(
        &path,
        &SecretBytes::new(serde_json::to_vec(&receipt).map_err(|_| invalid())?),
    )
    .map_err(|_| invalid())
}

fn invalid() -> io::Error {
    io::Error::other("native input stream contract failed")
}

#[cfg(test)]
mod tests;
