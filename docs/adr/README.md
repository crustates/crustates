# Architecture decision records

Each record follows the MADR template in
[`0000-template.md`](0000-template.md). To add one, see
[CONTRIBUTING.md](../../CONTRIBUTING.md#bigger-changes).

| No.  | Title                                                                                                                 | Status   |
| ---- | --------------------------------------------------------------------------------------------------------------------- | -------- |
| 0001 | [Reposition Crustates as a self-hosted homelab platform](0001-reposition-as-homelab-platform.md)                      | accepted |
| 0002 | [Base OS: Fedora bootc with ZFS and Btrfs](0002-base-os-fedora-bootc.md)                                              | accepted |
| 0003 | [No Kubernetes underneath; a Kubernetes-shaped API instead](0003-no-kubernetes-underneath.md)                         | accepted |
| 0004 | [Two compute resources, VM and App, with microVM isolation by default](0004-vm-and-app-with-microvm-isolation.md)     | accepted |
| 0005 | [Licence: AGPLv3 platform, Apache-2.0 SDKs and plugin interfaces, DCO](0005-licence-agpl-apache-dco.md)               | accepted |
| 0006 | [Full VMs on QEMU, microVMs on Cloud Hypervisor via Kata runtime-rs](0006-qemu-full-vms-cloud-hypervisor-microvms.md) | proposed |
| 0007 | [Ingress: Caddy with three certificate modes](0007-ingress-caddy-three-certificate-modes.md)                          | accepted |
| 0008 | [Default DNS zone `crust.internal` plus mDNS](0008-dns-zone-crust-internal-and-mdns.md)                               | accepted |
| 0009 | [Built-in accounts with passkeys, OIDC optional](0009-built-in-accounts-passkeys-oidc-optional.md)                    | proposed |
| 0010 | [Platform state in SQLite (Raft-replicated later), not PostgreSQL](0010-platform-state-in-sqlite.md)                  | accepted |
| 0011 | [AI assistant as an MCP client of the public API](0011-assistant-as-mcp-client.md)                                    | proposed |
| 0012 | [Spawn: pull-based GitOps plus push-to-deploy builds](0012-spawn-gitops-and-push-to-deploy.md)                        | accepted |
| 0013 | [Rust workspace layout](0013-rust-workspace-layout.md)                                                                | accepted |
