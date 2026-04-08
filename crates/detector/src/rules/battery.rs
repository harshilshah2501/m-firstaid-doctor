use crate::types::*;
use feature_extractor::FeatureSet;
use signal_normalizer::ThermalState;
use trigger_engine::IssueClass;

pub fn diagnose(f: &FeatureSet) -> Option<Diagnosis> {
    // Rule 1: Critical thermal state
    if f.thermal_state.as_ref().map(|s| *s >= ThermalState::Critical).unwrap_or(false)
        || f.cpu_temp_celsius.map(|t| t > 100.0).unwrap_or(false)
    {
        return Some(Diagnosis {
            issue_class: IssueClass::BatteryThermal,
            confidence: Confidence::Confirmed,
            severity: Severity::Critical,
            rule_name: "battery::thermal_critical".into(),
            title: format!(
                "Critical thermal state — CPU at {:?}°C",
                f.cpu_temp_celsius.map(|t| t as u32)
            ),
            explanation: "System temperature is at a critical level. The OS will begin \
                          aggressively throttling performance to protect hardware. \
                          Continued operation at this temperature risks permanent damage. \
                          Identify and stop the process causing thermal load immediately.".into(),
            evidence: vec![
                Evidence::field("thermal_state", format!("{:?}", f.thermal_state)),
                Evidence::field("cpu_temp_celsius", f.cpu_temp_celsius),
                Evidence::field("fan_speed_rpm", f.fan_speed_rpm),
            ],
            contributing_ml: None,
        });
    }

    // Rule 2: Battery health critically degraded
    if let Some(health) = f.battery_health_pct {
        if health < 70.0 {
            return Some(Diagnosis {
                issue_class: IssueClass::BatteryThermal,
                confidence: Confidence::Confirmed,
                severity: Severity::High,
                rule_name: "battery::health_critical".into(),
                title: format!("Battery health critically degraded: {:.0}%", health),
                explanation: format!(
                    "Battery holds only {:.0}% of its original design capacity. \
                     Apple recommends replacement below 80%. At {:.0}%, runtime will be \
                     significantly reduced and sudden shutdowns may occur at low charge levels. \
                     Battery replacement is recommended.",
                    health, health
                ),
                evidence: vec![
                    Evidence::field("battery_health_pct", health).with_threshold(80.0),
                    Evidence::field("battery_cycle_count", f.battery_cycle_count),
                ],
                contributing_ml: None,
            });
        }

        if health < 80.0 {
            return Some(Diagnosis {
                issue_class: IssueClass::BatteryThermal,
                confidence: Confidence::Confirmed,
                severity: Severity::Medium,
                rule_name: "battery::health_degraded".into(),
                title: format!("Battery health below optimal: {:.0}%", health),
                explanation: format!(
                    "Battery capacity has degraded to {:.0}% of original design capacity. \
                     Apple's threshold for replacement recommendation is 80%. \
                     Consider scheduling a battery service.",
                    health
                ),
                evidence: vec![
                    Evidence::field("battery_health_pct", health).with_threshold(80.0),
                    Evidence::field("battery_cycle_count", f.battery_cycle_count),
                ],
                contributing_ml: None,
            });
        }
    }

    // Rule 3: Elevated temperature (not yet critical)
    if let Some(temp) = f.cpu_temp_celsius {
        if temp > 90.0 {
            return Some(Diagnosis {
                issue_class: IssueClass::BatteryThermal,
                confidence: Confidence::Probable,
                severity: Severity::Medium,
                rule_name: "battery::thermal_elevated".into(),
                title: format!("Elevated CPU temperature: {:.0}°C", temp),
                explanation: format!(
                    "CPU temperature is {:.0}°C, which is elevated but not yet critical. \
                     Sustained temperatures above 90°C can shorten component lifespan. \
                     Ensure ventilation is not obstructed and check for CPU-intensive processes.",
                    temp
                ),
                evidence: vec![
                    Evidence::field("cpu_temp_celsius", temp).with_threshold(90.0),
                    Evidence::field("fan_speed_rpm", f.fan_speed_rpm),
                    Evidence::field("top_process_cpu_pct", f.top_process_cpu_pct)
                        .with_context(&f.top_process_name),
                ],
                contributing_ml: None,
            });
        }
    }

    None
}
