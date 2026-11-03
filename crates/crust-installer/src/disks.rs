//! The machine's disks, from `lsblk --json --bytes`.

use serde_json::Value;

use crate::hardware::Disk;

/// The `lsblk` columns [`parse`] reads.
pub const LSBLK: [&str; 4] = [
    "--json",
    "--bytes",
    "--output",
    "NAME,PATH,SIZE,MODEL,TRAN,TYPE,FSTYPE,RM,RO",
];

/// Whole disks from `lsblk` JSON, without CD-ROMs, loop and RAM devices.
/// `media` is the device the installer started from, if known.
#[must_use]
pub fn parse(json: &str, media: Option<&str>) -> Vec<Disk> {
    let tree: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    let devices = tree["blockdevices"].as_array().cloned().unwrap_or_default();
    devices
        .iter()
        .filter(|d| d["type"] == "disk" && d["ro"] != true)
        .filter_map(|d| {
            let path = d["path"].as_str()?.to_owned();
            let children = d["children"].as_array().cloned().unwrap_or_default();
            let kinds: Vec<&str> = children
                .iter()
                .filter_map(|c| c["fstype"].as_str())
                .collect();
            let contents = match (children.len(), d["fstype"].as_str()) {
                (0, None) => "empty".to_owned(),
                (0, Some(fs)) => format!("has data: {fs}"),
                (n, _) => {
                    let mut kinds = kinds.clone();
                    kinds.dedup();
                    let what = if kinds.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", kinds.join(", "))
                    };
                    format!(
                        "has data: {n} partition{}{what}",
                        if n == 1 { "" } else { "s" }
                    )
                }
            };
            let in_use =
                |p: &str| p == path || children.iter().any(|c| c["path"].as_str() == Some(p));
            Some(Disk {
                size: d["size"].as_u64().unwrap_or(0),
                model: d["model"].as_str().unwrap_or("-").trim().to_owned(),
                transport: match d["tran"].as_str() {
                    Some("nvme") => "NVMe".into(),
                    Some("sata") => "SATA".into(),
                    Some("usb") => "USB".into(),
                    Some(other) => other.to_uppercase(),
                    None if path.contains("/vd") => "virtio".into(),
                    None => "-".into(),
                },
                contents,
                install_media: media.is_some_and(in_use),
                path,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LSBLK_OUTPUT: &str = r#"{"blockdevices":[
        {"name":"sr0","path":"/dev/sr0","size":2147483648,"model":"QEMU DVD-ROM","tran":"sata","type":"rom","fstype":"iso9660","rm":true,"ro":false},
        {"name":"vda","path":"/dev/vda","size":42949672960,"model":null,"tran":null,"type":"disk","fstype":null,"rm":false,"ro":false},
        {"name":"sdb","path":"/dev/sdb","size":4000787030016,"model":"WD Red Plus  ","tran":"sata","type":"disk","fstype":null,"rm":false,"ro":false,
         "children":[{"name":"sdb1","path":"/dev/sdb1","fstype":"ntfs"},{"name":"sdb2","path":"/dev/sdb2","fstype":"ntfs"}]},
        {"name":"sdc","path":"/dev/sdc","size":32000000000,"model":"Stick","tran":"usb","type":"disk","fstype":null,"rm":true,"ro":false,
         "children":[{"name":"sdc1","path":"/dev/sdc1","fstype":"iso9660"}]}
    ]}"#;

    #[test]
    fn disks_say_what_is_on_them_and_which_one_is_the_stick() {
        let disks = parse(LSBLK_OUTPUT, Some("/dev/sdc1"));
        let names: Vec<&str> = disks.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(names, ["/dev/vda", "/dev/sdb", "/dev/sdc"], "no CD-ROM");
        assert_eq!(
            (disks[0].contents.as_str(), disks[0].transport.as_str()),
            ("empty", "virtio")
        );
        assert_eq!(disks[1].contents, "has data: 2 partitions (ntfs)");
        assert_eq!(disks[1].model, "WD Red Plus");
        assert!(disks[2].install_media && !disks[1].install_media);
    }
}
