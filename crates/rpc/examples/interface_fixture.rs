//! Run with `cargo run -p semantic_rpc --features plugin-stdio --example interface_fixture`.
//! All stdout belongs to the framed interface protocol.
use semantic_data::Value;
use semantic_rpc::interface::*;
use std::sync::Arc;

struct Fixture;
impl InterfaceImplementation for Fixture {
    fn descriptors(&self) -> &[ImplementationDescriptor] {
        &[]
    }
    fn invoke<'a>(
        &'a self,
        mut call: ValidatedInvocation,
        _: InvocationContext,
    ) -> InvocationFuture<'a> {
        Box::pin(async move {
            match call.arguments.pop() {
                Some(InvocationArgument::Stream(stream)) => Ok(InvocationOutput::Stream(stream)),
                Some(InvocationArgument::Value(value)) => Ok(InvocationOutput::Values(vec![value])),
                None => Ok(InvocationOutput::Stream(OwnedValueStream::new(
                    futures::stream::iter([
                        Ok(StreamEvent::Item(Value::U64(u64::MAX))),
                        Ok(StreamEvent::End(None)),
                    ]),
                ))),
            }
        })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), InvocationError> {
    if std::env::args().any(|arg| arg == "--crash-on-invoke" || arg == "--exit-after-ready") {
        return crash_on_invoke().await;
    }
    semantic_rpc::plugin::stdio::serve(
        tokio::io::stdin(),
        tokio::io::stdout(),
        Arc::new(Fixture),
        None,
        |_| async { Ok(()) },
    )
    .await
}

// Echo the host's declarations so lifecycle tests can use their own manifest.
async fn crash_on_invoke() -> Result<(), InvocationError> {
    use semantic_rpc::plugin::stdio::{read_frame, write_frame};
    use semantic_rpc_core::interface_protocol::InterfaceMessage;
    let mut input = tokio::io::BufReader::new(tokio::io::stdin());
    let mut output = tokio::io::stdout();
    while let Some(frame) = read_frame(&mut input).await? {
        let message: InterfaceMessage = serde_json::from_slice(&frame).unwrap();
        let reply = match message {
            hello @ InterfaceMessage::Hello { .. } => hello,
            InterfaceMessage::Configure { .. } => InterfaceMessage::Ready,
            InterfaceMessage::Shutdown => {
                write_frame(
                    &mut output,
                    &serde_json::to_vec(&InterfaceMessage::ShutdownAck).unwrap(),
                )
                .await?;
                return Ok(());
            }
            InterfaceMessage::Call { .. } => std::process::exit(17),
            _ => continue,
        };
        write_frame(&mut output, &serde_json::to_vec(&reply).unwrap()).await?;
        if matches!(reply, InterfaceMessage::Ready)
            && std::env::args().any(|arg| arg == "--exit-after-ready")
        {
            std::thread::sleep(std::time::Duration::from_millis(100));
            std::process::exit(17);
        }
    }
    Ok(())
}
