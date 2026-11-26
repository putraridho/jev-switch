//! Allow-list for DeepSeek balance responses.
use crate::meter::{BalanceReading, Meter, Meters};
use crate::quantity::{Currency, Money};
use crate::usage::Unavailable;
use serde::Deserialize;

#[derive(Deserialize)]
struct Response<'a> {
    is_available: bool,
    #[serde(borrow)]
    balance_infos: Vec<BalanceData<'a>>,
}
#[derive(Deserialize)]
struct BalanceData<'a> {
    currency: &'a str,
    total_balance: &'a str,
    granted_balance: Option<&'a str>,
    topped_up_balance: Option<&'a str>,
}
/// Parse one balance per currency.
pub fn parse_balance(body: &[u8]) -> Result<Meters, Unavailable> {
    let response: Response<'_> = serde_json::from_slice(body).map_err(|_| Unavailable::Protocol)?;
    let mut meters = Vec::new();
    for balance in response.balance_infos {
        let currency = match balance.currency {
            "USD" => Currency::Usd,
            "CNY" => Currency::Cny,
            _ => Currency::Other,
        };
        if meters
            .iter()
            .any(|m: &Meter| m.id() == crate::meter::MeterId::Balance(currency))
        {
            continue;
        }
        let Some(total) = Money::parse_decimal(balance.total_balance) else {
            continue;
        };
        meters.push(Meter::Balance(BalanceReading {
            currency,
            total,
            granted: balance.granted_balance.and_then(Money::parse_decimal),
            topped_up: balance.topped_up_balance.and_then(Money::parse_decimal),
            usable: response.is_available,
        }));
        if meters.len() == Meters::MAX {
            break;
        }
    }
    Meters::new(meters).ok_or(Unavailable::Protocol)
}
