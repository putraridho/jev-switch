//! The door to the Jev service. Real adapter: `infrastructure::typesafe_http::TypeSafe`;
//! tests may use fakes.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use serde_json::Value;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait JevTransport: Send + Sync {
    /// Several judgments in ONE round trip. `questions` is a JSON object keyed by
    /// name (build each value with `wire::choice_question` / `wire::noul_question`); the
    /// result is the `answers` object with the same keys, read with `wire::decode_choice`
    /// and `wire::decode_noul`. Every question shares `state`, and the API answers them
    /// together, so N judgments cost about one request's latency instead of N.
    fn ask_many(&self, state: Value, questions: Value) -> BoxFut<'_, Result<Value>>;
}
