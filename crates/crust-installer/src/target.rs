//! The files the installer writes into the new system: the management
//! network, the Molt channel and policy, the hostname.

use crate::answers::Answers;

/// A NetworkManager keyfile for a fixed address or a VLAN; `None` for
/// plain DHCP, which NetworkManager does by itself on every wired port.
#[must_use]
pub fn keyfile(answers: &Answers, interface: &str) -> Option<String> {
    let net = &answers.network;
    if !net.is_static() && net.vlan.is_none() {
        return None;
    }
    let mut out = String::from(
        "# Written by crust-installer: the management network.\n[connection]\nid=crust-mgmt\n",
    );
    match net.vlan {
        Some(id) => out.push_str(&format!(
            "type=vlan\ninterface-name={interface}.{id}\nautoconnect=true\n\n[vlan]\nparent={interface}\nid={id}\n"
        )),
        None => out.push_str(&format!("type=ethernet\ninterface-name={interface}\nautoconnect=true\n")),
    }
    out.push_str("\n[ipv4]\n");
    if net.is_static() {
        out.push_str("method=manual\n");
        if net.gateway.is_empty() {
            out.push_str(&format!("address1={}\n", net.address));
        } else {
            out.push_str(&format!("address1={},{}\n", net.address, net.gateway));
        }
        if !net.dns.is_empty() {
            out.push_str(&format!("dns={};\n", net.dns.join(";")));
        }
    } else {
        out.push_str("method=auto\n");
    }
    out.push_str("\n[ipv6]\nmethod=auto\n");
    Some(out)
}

/// `/etc/crustates/molt.toml`: the channel and the update policy (ADR 0016,
/// ADR 0017).
#[must_use]
pub fn molt_toml(answers: &Answers) -> String {
    format!(
        "# Written by crust-installer; the image's defaults are in /usr/lib/crustates/molt.toml.\nchannel = \"{}\"\npolicy = \"{}\"\n",
        answers.channel, answers.policy
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answers::NetworkAnswers;

    #[test]
    fn dhcp_needs_no_keyfile_static_and_vlans_do() {
        let mut answers = Answers::default();
        assert_eq!(keyfile(&answers, "enp1s0"), None);
        answers.network = NetworkAnswers {
            method: "static".into(),
            address: "10.0.10.2/24".into(),
            gateway: "10.0.10.1".into(),
            dns: vec!["10.0.10.53".into(), "9.9.9.9".into()],
            vlan: Some(10),
            ..NetworkAnswers::default()
        };
        let file = keyfile(&answers, "enp1s0").unwrap();
        for line in [
            "type=vlan",
            "interface-name=enp1s0.10",
            "parent=enp1s0",
            "address1=10.0.10.2/24,10.0.10.1",
            "dns=10.0.10.53;9.9.9.9;",
        ] {
            assert!(file.contains(line), "{line}: {file}");
        }
        answers.channel = "beta".into();
        answers.policy = "manual".into();
        assert!(molt_toml(&answers).ends_with("channel = \"beta\"\npolicy = \"manual\"\n"));
    }
}
