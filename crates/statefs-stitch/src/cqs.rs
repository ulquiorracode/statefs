//! Standard CQS (Command-Query Separation) abstractions for StateFS.
//!
//! Enforces at compile-time that:
//! - Queries inspect state immutably (`&Ctx`) and cannot cause side effects.
//! - Commands mutate state exclusively (`&mut Ctx`) and return receipts or errors.

/// Pure Query contract: inspects context immutably without side effects.
pub trait Query<Ctx> {
    /// Resulting data returned by this query.
    type Output;

    /// Evaluates the query against an immutable context.
    fn execute(&self, ctx: &Ctx) -> Self::Output;
}

/// Pure Command contract: requests state mutation with explicit receipts or failure.
pub trait Command<Ctx> {
    /// Receipt emitted on successful mutation.
    type Receipt;

    /// Error emitted if the mutation invariant or policy fails.
    type Error;

    /// Executes the mutation against a mutable context.
    fn execute(&mut self, ctx: &mut Ctx) -> Result<Self::Receipt, Self::Error>;
}
