//! The body of DeepSeek's `GET /user/balance` (DESIGN 6.4). T01 writes the body.

use crate::meter::Meters;
use crate::usage::Unavailable;

/// One balance meter per currency.
pub fn parse_balance(body: &[u8]) -> Result<Meters, Unavailable> {
    let _ = body;
    Err(Unavailable::Internal)
}
