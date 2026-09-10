//! Real child-process fixture for the provider acceptance test.
#[path = "../tests/support/mod.rs"]
mod support;
use semantic_rpc::interface::*;
use std::sync::Arc;
struct Fixture(Arc<dyn InterfaceImplementation>);
impl InterfaceImplementation for Fixture {
    fn descriptors(&self) -> &[ImplementationDescriptor] {
        self.0.descriptors()
    }
    fn invoke<'a>(
        &'a self,
        call: ValidatedInvocation,
        context: InvocationContext,
    ) -> InvocationFuture<'a> {
        if call.method == "exit" {
            std::process::exit(0);
        }
        self.0.invoke(call, context)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), semantic_rpc::interface::InvocationError> {
    semantic_rpc::plugin::stdio::serve(
        tokio::io::stdin(),
        tokio::io::stdout(),
        Arc::new(Fixture(support::implementation().await)),
        Some("1".into()),
        |_| async { Ok(()) },
    )
    .await
}
