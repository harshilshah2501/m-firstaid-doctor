use crate::types::*;
use feature_extractor::FeatureSet;
use trigger_engine::IssueClass;

pub fn diagnose(f: &FeatureSet) -> Option<Diagnosis> {
    // Rule 1: Complete disconnection
    if !f.network_reachable {
        return Some(Diagnosis {
            issue_class: IssueClass::NetworkDns,
            confidence: Confidence::Confirmed,
            severity: Severity::High,
            rule_name: "network::no_reachability".into(),
            title: "Network unreachable — no internet connectivity".into(),
            explanation: "The system has no active network interface with internet reachability. \
                          This may be caused by WiFi/Ethernet being disconnected, a failed DHCP \
                          lease, or a proxy/VPN blocking all traffic.".into(),
            evidence: vec![
                Evidence::field("network_reachable", f.network_reachable),
                Evidence::field("vpn_active", f.vpn_active),
                Evidence::field("proxy_configured", f.proxy_configured),
                Evidence::field("wifi_signal_dbm", f.wifi_signal_dbm),
            ],
            contributing_ml: None,
        });
    }

    // Rule 2: DNS failure
    if f.dns_resolution_fail_rate > 0.5 {
        let (confidence, title, explanation) = if f.vpn_active {
            (
                Confidence::Probable,
                "DNS resolution failing — possible VPN split-tunneling issue",
                "More than 50% of DNS lookups are failing. A VPN is active, which commonly \
                 causes DNS resolution failures when the VPN's DNS server is unreachable or \
                 split-tunneling is misconfigured.",
            )
        } else if f.proxy_configured {
            (
                Confidence::Probable,
                "DNS resolution failing — proxy may be blocking DNS",
                "More than 50% of DNS lookups are failing. An HTTP proxy is configured, which \
                 may be interfering with DNS resolution.",
            )
        } else {
            (
                Confidence::Confirmed,
                "DNS resolution failure — configured DNS server is not responding",
                "More than 50% of DNS lookups are failing and no VPN or proxy is active. \
                 The configured DNS server is likely down or unreachable. \
                 Falling back to 8.8.8.8 (Google DNS) may restore connectivity.",
            )
        };

        return Some(Diagnosis {
            issue_class: IssueClass::NetworkDns,
            confidence,
            severity: Severity::High,
            rule_name: "network::dns_failure".into(),
            title: title.into(),
            explanation: explanation.into(),
            evidence: vec![
                Evidence::field("dns_resolution_fail_rate", f.dns_resolution_fail_rate)
                    .with_threshold(0.5),
                Evidence::field("vpn_active", f.vpn_active),
                Evidence::field("proxy_configured", f.proxy_configured),
            ],
            contributing_ml: None,
        });
    }

    // Rule 3: Intermittent DNS failure
    if f.dns_resolution_fail_rate > 0.2 {
        return Some(Diagnosis {
            issue_class: IssueClass::NetworkDns,
            confidence: Confidence::Probable,
            severity: Severity::Medium,
            rule_name: "network::dns_intermittent".into(),
            title: format!(
                "Intermittent DNS failures ({:.0}% failure rate)",
                f.dns_resolution_fail_rate * 100.0
            ),
            explanation: "DNS resolution is intermittently failing. This may indicate \
                          an unstable connection, overloaded DNS server, or packet loss \
                          on the network path.".into(),
            evidence: vec![
                Evidence::field("dns_resolution_fail_rate", f.dns_resolution_fail_rate)
                    .with_threshold(0.2),
                Evidence::field("default_gateway_ping_ms", f.default_gateway_ping_ms),
            ],
            contributing_ml: None,
        });
    }

    // Rule 4: Weak WiFi signal
    if let Some(dbm) = f.wifi_signal_dbm {
        if dbm < -80 {
            return Some(Diagnosis {
                issue_class: IssueClass::NetworkDns,
                confidence: Confidence::Probable,
                severity: Severity::Medium,
                rule_name: "network::weak_wifi".into(),
                title: format!("Weak WiFi signal: {} dBm", dbm),
                explanation: format!(
                    "WiFi signal strength is {} dBm, which is below the reliable threshold \
                     of -80 dBm. Packet loss and intermittent connectivity are likely. \
                     Moving closer to the router or using Ethernet will resolve this.", dbm
                ),
                evidence: vec![
                    Evidence::field("wifi_signal_dbm", dbm).with_threshold(-80i32),
                ],
                contributing_ml: None,
            });
        }
    }

    None
}
