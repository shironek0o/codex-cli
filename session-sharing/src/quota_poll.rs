use std::time::Duration;

use serde_json::Value;

const RESET_GRACE_SECS: u64 = 30;
const MIN_WAIT_SECS: u64 = 60;
const MAX_WAIT_SECS: u64 = 15 * 60;

#[derive(Debug, PartialEq, Eq)]
pub enum QuotaCheck {
    Ready,
    Wait(Duration),
}

pub fn check_quota(response: &Value, model: Option<&str>, now: u64) -> QuotaCheck {
    evaluate(response, model, now).unwrap_or(QuotaCheck::Wait(Duration::from_secs(MIN_WAIT_SECS)))
}

fn evaluate(response: &Value, model: Option<&str>, now: u64) -> Option<QuotaCheck> {
    let default = response.get("rateLimits")?;
    let additional = response["rateLimitsByLimitId"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, snapshot)| {
            model.is_some_and(|model| *key == model || snapshot["normalModelSlug"] == model)
        })
        .map(|(_, snapshot)| snapshot);
    let mut blocked = false;
    let mut latest_reset = now;
    for snapshot in std::iter::once(default).chain(additional) {
        let snapshot = snapshot.as_object()?;
        match snapshot.get("spendControlReached").unwrap_or(&Value::Null) {
            Value::Bool(true) => blocked = true,
            Value::Null | Value::Bool(false) => {}
            _ => return None,
        }
        let mut observed = false;
        for window in ["primary", "secondary"]
            .into_iter()
            .filter_map(|key| snapshot.get(key).filter(|value| !value.is_null()))
        {
            let used = window["usedPercent"].as_f64()?;
            if !(0.0..=100.0).contains(&used) {
                return None;
            }
            observed = true;
            if used >= 100.0 {
                blocked = true;
                let reset = window["resetsAt"].as_u64()?;
                if reset <= now {
                    return None;
                }
                latest_reset = latest_reset.max(reset);
            }
        }
        if !observed {
            return None;
        }
    }
    // A reset timestamp or an apparently available window alone is not permission to resume.
    if !blocked && response["ordinaryUsageAllowed"] == true {
        return Some(QuotaCheck::Ready);
    }
    let delay = latest_reset
        .saturating_sub(now)
        .saturating_add(RESET_GRACE_SECS)
        .clamp(MIN_WAIT_SECS, MAX_WAIT_SECS);
    Some(QuotaCheck::Wait(Duration::from_secs(delay)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: u64 = 1000;

    #[test]
    fn waits_for_both_exhausted_windows_and_rechecks_during_long_waits() {
        let mut response = json!({"ordinaryUsageAllowed": false, "rateLimits": {
            "primary": {"usedPercent": 100, "resetsAt": 1100},
            "secondary": {"usedPercent": 100, "resetsAt": 1200},
        }});
        assert_eq!(
            check_quota(&response, None, NOW),
            QuotaCheck::Wait(Duration::from_secs(230))
        );
        response["rateLimits"]["secondary"]["resetsAt"] = json!(5000);
        assert_eq!(
            check_quota(&response, None, NOW),
            QuotaCheck::Wait(Duration::from_secs(900))
        );
        response["rateLimits"]["secondary"]["resetsAt"] = json!(NOW);
        assert_eq!(
            check_quota(&response, None, NOW),
            QuotaCheck::Wait(Duration::from_secs(60))
        );
    }

    #[test]
    fn waits_for_authoritative_included_usage_permission() {
        let mut response = json!({"rateLimits": {"primary": {"usedPercent": 1}}});
        for allowed in [Value::Null, json!(false)] {
            response["ordinaryUsageAllowed"] = allowed;
            assert_eq!(
                check_quota(&response, None, NOW),
                QuotaCheck::Wait(Duration::from_secs(60))
            );
        }
        response["ordinaryUsageAllowed"] = json!(true);
        assert_eq!(check_quota(&response, None, NOW), QuotaCheck::Ready);
        response["rateLimits"] = json!({});
        assert_eq!(
            check_quota(&response, None, NOW),
            QuotaCheck::Wait(Duration::from_secs(60))
        );
    }

    #[test]
    fn consults_only_the_applicable_additional_model_quota() {
        let response = json!({
            "ordinaryUsageAllowed": true,
            "rateLimits": {"primary": {"usedPercent": 20}},
            "rateLimitsByLimitId": {
                "separate-bucket": {"normalModelSlug": "model-a", "primary": {"usedPercent": 100, "resetsAt": 1500}},
            },
        });
        assert_eq!(
            check_quota(&response, Some("model-b"), NOW),
            QuotaCheck::Ready
        );
        assert_eq!(
            check_quota(&response, Some("model-a"), NOW),
            QuotaCheck::Wait(Duration::from_secs(530))
        );
    }
}
