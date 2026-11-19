//! DeepSeek account balances, kept separate from UI so callers can choose when to fetch them.

use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
struct BalanceResponse {
    is_available: bool,
    balance_infos: Vec<BalanceInfo>,
}

#[derive(Deserialize)]
struct BalanceInfo {
    currency: String,
    total_balance: String,
}

/// The selected currency's credit and whether the account can use it.
#[derive(Debug, Clone, PartialEq)]
pub struct Balance {
    pub currency: String,
    pub total: f64,
    pub available: bool,
}

/// Prefer USD so the displayed balance is stable when multiple currencies are returned.
pub fn parse_balance(body: &str) -> Option<Balance> {
    let response: BalanceResponse = serde_json::from_str(body).ok()?;
    let info = response
        .balance_infos
        .iter()
        .find(|info| info.currency == "USD")
        .or_else(|| response.balance_infos.first())?;
    let total: f64 = info.total_balance.parse().ok()?;
    if !total.is_finite() {
        return None;
    }
    Some(Balance {
        currency: info.currency.clone(),
        total,
        available: response.is_available,
    })
}

/// Format the selected currency for a compact credit indicator.
pub fn format_balance(b: &Balance) -> String {
    let amount = match b.currency.as_str() {
        "USD" => format!("${:.2}", b.total),
        "CNY" => format!("¥{:.2}", b.total),
        currency => format!("{:.2} {currency}", b.total),
    };
    let unavailable = if b.available { "" } else { " (unavailable)" };
    format!("{amount} left{unavailable}")
}

/// Fetch the account balance with a short timeout so a credit indicator cannot stall a turn.
pub async fn fetch_deepseek_balance(key: &str) -> Option<Balance> {
    fetch_balance_from("https://api.deepseek.com", key).await
}

/// Accept a base URL so callers can exercise the request against a local server.
pub async fn fetch_balance_from(base_url: &str, key: &str) -> Option<Balance> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;
    let response = client
        .get(format!("{}/user/balance", base_url.trim_end_matches('/')))
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    parse_balance(&response.text().await.ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usd_is_preferred() {
        let body = r#"{"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"12.50"},{"currency":"USD","total_balance":"110.00"}]}"#;
        assert_eq!(
            parse_balance(body),
            Some(Balance {
                currency: "USD".into(),
                total: 110.0,
                available: true
            })
        );
    }

    #[test]
    fn cny_only() {
        let body = r#"{"is_available":false,"balance_infos":[{"currency":"CNY","total_balance":"12.50"}]}"#;
        assert_eq!(
            parse_balance(body),
            Some(Balance {
                currency: "CNY".into(),
                total: 12.5,
                available: false
            })
        );
    }

    #[test]
    fn invalid_balances() {
        assert_eq!(
            parse_balance(r#"{"is_available":true,"balance_infos":[]}"#),
            None
        );
        assert_eq!(
            parse_balance(
                r#"{"is_available":true,"balance_infos":[{"currency":"USD","total_balance":"bad"}]}"#
            ),
            None
        );
        assert_eq!(parse_balance("{broken"), None);
    }

    #[test]
    fn formats_currencies_and_availability() {
        assert_eq!(
            format_balance(&Balance {
                currency: "USD".into(),
                total: 110.0,
                available: true
            }),
            "$110.00 left"
        );
        assert_eq!(
            format_balance(&Balance {
                currency: "CNY".into(),
                total: 12.5,
                available: false
            }),
            "¥12.50 left (unavailable)"
        );
        assert_eq!(
            format_balance(&Balance {
                currency: "EUR".into(),
                total: 12.5,
                available: true
            }),
            "12.50 EUR left"
        );
    }
}
