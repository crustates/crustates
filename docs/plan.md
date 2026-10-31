# Crustates — homelab platform plan

Oct 30, 2026 · @Miguel Martens

## Vision and positioning

Crustates is your own sovereign cloud on your own hardware: one Rust platform for compute, storage and networking on amd64 and arm64, managed entirely through an API. It replaces the Proxmox + TrueNAS + Portainer stack in a homelab with one coherent product.

**Who it's for:** homelab and self-hosting enthusiasts, small offices and edge sites that want VMs, apps and storage without stitching together five tools.

**Why it's different**

- **Secure by default:** every app runs in its own microVM, the OS is read-only and image-based, and nothing is reachable without the API.
- **API-first:** the web UI, CLI, TUI, OpenTofu provider and MCP server are all just clients of one declarative API, documented with an interactive Scalar reference generated from the OpenAPI spec.
- **Multi-site without port forwarding:** nodes at home, at a relative's place and on a VPS form one cluster.
- **eBPF-native networking:** per-workload firewalling, flow logs and metrics out of the box.
- **Safe plugins:** extensions run as sandboxed Wasm components.
- **An assistant that knows your box:** ask questions about the setup in plain language and let it act, with a local model by default and every change shown as a plan you confirm.

**Principles**

1. Reuse proven open source for the engines; write Rust for the control plane, agent and glue.
2. Every external tool sits behind a trait with contract tests, so it can be swapped.
3. Declarative resources (`spec` + `status`) and reconcile loops, Kubernetes-shaped but without Kubernetes underneath.
4. A boring core with a few deliberate innovations, each behind a feature flag until proven.
5. Fully open source and self-hosted; no cloud account required.

## Names and brand

The Crustates brand carries over from the earlier plan; only the meaning of the product names changes. Every product area gets its own icon and accent colour, drawn in the same blocky style as the crab.

| Name | Area | Covers | Accent | CLI example |
| --- | --- | --- | --- | --- |
| Crustates | Platform | Everything | Existing palette | `crust` |
| Tide | Compute | VMs and apps (microVMs, containers) | EU blue `#1F3FD1` | `crust tide vm create` |
| Reef | Storage | Pools, shares, S3, snapshots, backups | Coral `#E8552D` | `crust reef pool create` |
| Current | Networking | Bridges, VLANs, eBPF policy, firewall, mesh, remote access | Sea-teal `#0E8A7E`, darkened to #0B7A70 for contrast | `crust current net create` |
| Molt | OS lifecycle | Atomic upgrades and rollback | Yellow #FFD23F | `crust molt` |
| Cast | Cluster | Joining nodes and sites | Driftwood brown #8B5E3C | `crust cast join` |
| Barnacle | Plugins | Sandboxed Wasm extensions that attach to the platform | Violet `#A77BFF` | `crust barnacle install` |
| Spawn | GitOps and deploys | Git sources, drift, PR plans, push-to-deploy builds and previews | Pink `#FF7AA8` | `crust spawn sync` |

- Reuse `@crustates/brand` (tokens, fonts, assets) and `@crustates/ui` (neobrutalist components) as they are.
- The brand kit (logo, product icons, badges, banners, generator) lives in the public `crustates/brand` repo.
- Update positioning copy, TRADEMARK.md subtitles and baked-in assets (OG image, badges) from "hosted cloud" to "your own cloud, on your own hardware".
- Code names stay plain (`vm-driver`, `net-driver`); theme names are for things users see.

## Architecture

&#91;embedded content: Crustates architecture · clients, control plane, node agent, engines, OS\]

All Rust code sits in the control plane and the node agent; every driver wraps a proven engine behind a trait, so engines can be swapped. Wasm plugins load into both layers.

## Areas in detail

### Molt: the operating system

- A bootc image: the whole OS, kernel and initrd (or UKI) shipped as an OCI image, built from a Containerfile and pulled from a registry.
- Read-only root with composefs and fs-verity; atomic A/B upgrades with rollback (`crust molt`).
- Base: Fedora bootc with Fedora's own kernel pinned per release (a separately built LTS kernel is a later option). Btrfs is built into that kernel; the ZFS module is compiled into the image in CI against the exact same kernel, so an image only ships if ZFS builds and both pool types pass tests. This is the approach uCore already uses.
- No SSH by default: everything goes through the API.
- Watch: Live Update Orchestrator (systemd 262) for host kernel updates without stopping VMs. Experimental, not in v1.
- Installer: a bootable USB/ISO image (bootc-image-builder), a first-run wizard for disks, network and the admin account, and a hardware compatibility check.
- Local recovery console on the machine's own screen, like ESXi's: shows the IP, resets networking, rolls back with Molt, resets the admin password.
- Hardware requirements: documented minimum CPU and RAM, checks that virtualization (VT-x or AMD-V) and the IOMMU are enabled, and a list of supported arm64 boards. Whether the image boots on a Raspberry Pi is still to be verified.
- Update delivery: Molt and platform images published to a container registry with stable and beta channels, plus offline updates from a USB stick.

### Tide: compute

Users see two resources; isolation is a setting, not a separate product.

| User starts | Resource | Default runtime | Option |
| --- | --- | --- | --- |
| An ISO or disk image (Windows, Home Assistant OS, OPNsense) | VM | QEMU with KVM: UEFI, vTPM, VNC console, USB | Cloud Hypervisor for headless Linux VMs |
| An OCI image or Compose app | App | MicroVM via Kata Containers (runtime-rs) on Cloud Hypervisor | `isolation: shared` runs a plain container |

- Full VMs stay for Windows, ISO installers (OPNsense, Home Assistant OS), GPU and PCI passthrough, Secure Boot and vTPM.
- Cloud Hypervisor on arm64 needs a GICv3-ITS interrupt controller, which no Raspberry Pi has; on such boards microVMs fall back to QEMU's microvm machine type or to shared isolation.
- Containers: containerd is the only container manager; per app it picks Kata Containers runtime-rs (microVM, default), crun (shared isolation) or later runwasi (Wasm). youki can replace crun once proven. Compose files are imported into App resources.
- Kubernetes is a workload, not the underlayer: a one-click k3s or Talos cluster built from VMs.
- Windows guests: the virtio driver ISO is bundled, and a virtual TPM (swtpm) with Secure Boot lets Windows 11 install.
- Consoles: noVNC or SPICE in the browser for VMs; logs and an interactive shell for apps.
- VM essentials: cloud-init, templates, an ISO library, and USB, disk and GPU passthrough, including a shared iGPU for media transcoding.
- App catalog: one-click apps from curated, community-contributed templates, in a Compose-based format of our own with importers for existing catalogs.
- App updates: detect new image versions; update manually or automatically, with rollback.
- Secrets: a built-in store that injects secrets into apps and into VMs via cloud-init.

### Reef: storage

- Pools: ZFS and Btrfs, both first-class, behind a storage trait; bcachefs as an opt-in experimental driver only.
- Shares: Samba (SMB) and NFS. Object storage: Garage (S3-compatible, Rust).
- Snapshots, replication between nodes, and backups with rustic (Rust).
- VM disks and app volumes come from pools; virtio-fs shares host directories into VMs.
- Disk health: SMART monitoring and scheduled scrubs, with alerts.
- Workload backups: scheduled, incremental, deduplicated backups of VMs and apps with restore, comparable to Proxmox Backup Server.
- Platform backup: export and restore of the control plane's state and config, so a dead boot disk isn't fatal.
- Encryption at rest: ZFS native encryption, LUKS under Btrfs, unlocked by the TPM.
- Databases for apps: PostgreSQL ships first as a catalog app. Later, a Database resource (PostgreSQL first, then MariaDB and Valkey) runs as an App microVM on a pool; Crustates handles creation, upgrades and users, continuous WAL backups with point-in-time restore (pgBackRest or WAL-G) to Garage, and injects connection strings into apps as secrets. CloudNativePG is not an option because it needs Kubernetes.

### Current: networking

- Bridges, VLANs and bonds via netlink.
- Host firewall in nftables (simple to audit, active before the agent starts).
- Workload data path in eBPF with Aya on tc and netkit hooks: per-workload policy, flow logs and metrics. XDP fast path later.
- Node-to-node fabric: WireGuard managed by the control plane; iroh (QUIC, dial by public key, NAT traversal) for the control channel across sites.
- Remote access as a pluggable `AccessProvider`: NetBird first, Tailscale/Headscale second.
- Built-in DNS integration point (goethite fits here).
- IP management: DHCP and IPAM for VMs and apps, IPv4 and IPv6.
- Ingress: Caddy as the built-in reverse proxy, configured through its admin API by the Current driver, with local DNS names such as `jellyfin.crust.internal`.
- Certificates, three modes:
  1. Local CA (default, works offline): Caddy's built-in CA signs internal names; the root certificate is shared to devices by QR code or download page.
  2. Let's Encrypt via DNS-01: real certificates for internal names under a domain you own, with nothing exposed; the Molt image ships a Caddy build with common DNS providers.
  3. Let's Encrypt via HTTP-01: for apps you deliberately expose to the internet.
- Layers: eBPF and nftables handle network traffic, Caddy handles HTTP routing and TLS, the DNS integration (goethite) makes names resolve.
- Names: a DNS zone set in the first-run wizard and changeable later, `crust.internal` by default. ICANN has reserved `.internal` for private use and will never delegate it; `home.arpa` works too. Avoid a bare made-up TLD like `.crust`, which could be delegated publicly one day.
- Every app and VM gets `<name>.crust.internal` automatically, signed by the local CA. With your own domain set (for example `home.example.com`), Crustates switches to Let's Encrypt via DNS-01.
- mDNS: apps are also announced as `<name>.local`, so they resolve on the LAN without any DNS setup.

### Cast: clustering

- Single node first, with SQLite as the state store: embedded in the binary, nothing extra to run or upgrade, and platform backup is one file. PostgreSQL would only pay off with heavy concurrent writes, which a homelab doesn't have; Incus takes the same route with Raft-replicated SQLite (cowsql).
- Multi-node later: Raft (openraft) for state with SQLite underneath, nodes joined with `crust cast join`, sites connected over iroh.

### Barnacle: plugins

- Wasmtime embedded in the agent and control plane; plugins are Wasm components with typed WIT interfaces.
- Use cases: custom drivers, event hooks (on VM created, register DNS), policy checks, dashboard widgets.
- Each plugin gets only the capabilities it is granted, so a bad plugin cannot take down the host.
- Distribution: plugins come from a registry, are signed, and the user approves each plugin's permissions on install.
- CLI and manifest: `crust barnacle install|new|build|dev`, with each plugin described in a `barnacle.toml` (the capabilities it asks for and the hooks it registers). The name fits the theme: a barnacle attaches itself to a host and lives off what it is given.

### Spawn: GitOps and push-to-deploy

Deploy from Git, two ways. Spawn is pull-based, so nothing on the internet needs to reach the box.

**GitOps for your whole setup**

- A `GitSource` points at a repo, branch and folder: GitHub (through a GitHub App), Forgejo/Gitea, GitLab, or plain git over https/ssh with a deploy key. Crustates pulls every few minutes and on webhooks (relayed, no port forwarding).
- The folder holds YAML for any resource in the API: apps, VMs, networks, firewall rules, shares, backup policies. The reconcilers apply it exactly like a CLI or UI change.
- Drift detection: changes made in the UI show as drift, with "revert to Git" or "adopt into Git (open a PR)". A prune policy decides whether resources removed from Git are deleted or only flagged.
- Plan on every pull request: the GitHub App posts the dry-run diff as a check, the same plan the assistant shows. Merge to apply; `git revert` to roll back.
- Secrets stay encrypted in the repo with SOPS + age; the private key never leaves the node.
- Image update PRs (Renovate-style) for apps that pin versions.
- For VMs, Git manages the spec (size, disks, network, cloud-init); what runs inside stays with cloud-init or Ansible.

**Push-to-deploy for your own code**

- Connect an app repo; every push builds on the box with BuildKit, from a Dockerfile or zero-config with Railpack, into a local registry, then rolls out with health checks and one-click rollback.
- Preview apps per pull request at `pr-42.<app>.crust.internal`, removed when the PR closes.
- Optional self-hosted GitHub Actions or Forgejo runner as an app in its own microVM.

**Safety**: Git changes run through the same role checks and plan as everything else, audit entries are tagged "via git" with the commit, and a source can be paused.

### Hardware support

Crustates runs Fedora's kernel, pinned per release, so hardware support is Fedora's: everything in the mainline kernel plus linux-firmware. Anything outside the kernel is built and signed in our CI, the same way as ZFS, and only ships after the image boots in a VM-in-CI test.

**Specs**

| Part | Minimum (it runs) | Recommended (typical homelab) | With a local AI model |
| --- | --- | --- | --- |
| CPU | 64-bit, 2 cores, VT-x / AMD-V | 6–8 cores, VT-d / AMD-Vi (IOMMU); Intel Quick Sync for media | Same, or 8+ cores |
| Memory | 8 GB | 32 GB, ECC if the board supports it | 48–64 GB, or a GPU |
| Boot disk | 64 GB SSD | 256 GB NVMe, separate from data | Same |
| Data disks | Optional (small pool on the boot disk) | 2+ matching disks in a mirror | Same |
| Network | 1 GbE, wired | 2.5 GbE, a second port for VLANs | Same |
| Firmware | UEFI | UEFI with Secure Boot and TPM 2.0 | Same |
| GPU | None | iGPU for transcoding | 8–12 GB VRAM for 7–14B models |
| Example | N100/N150 mini PC, 16 GB | Ryzen 5/7 or 12th-gen+ Core i5, 32 GB, two disks | Recommended box plus a GPU |

- Without VT-x/AMD-V there are no VMs or microVMs; apps fall back to shared containers (the installer's hardware check says so).
- Memory budget: platform about 1 GB, ZFS cache a few GB (given back under pressure), each app microVM tens of MB on top of the app, a Windows VM 4–8 GB.
- IOMMU and TPM are recommended, not required: without them you lose passthrough and automatic unlock of encrypted pools, not the install.
- Raspberry Pi 5 (8–16 GB) stays Community: no microVMs, microSD boot.

| Area | Support | Notes |
| --- | --- | --- |
| Network | Supported | Intel 1G, i225/i226 2.5G, X540/X710 10G; Realtek 2.5G/5G; Aquantia/Marvell 10G; Mellanox ConnectX; Broadcom. Wi-Fi works but is not recommended for the host's main link |
| Storage controllers | Supported | SATA/AHCI, NVMe, LSI/Broadcom HBAs in IT mode, USB disks for backups. RAID cards in HBA/JBOD mode so ZFS and Btrfs see the disks |
| Intel GPU | Supported | Quick Sync transcoding, sharing with containers and whole-GPU passthrough. SR-IOV only where the stock driver supports it (Nova Lake via xe from Linux 7.2); older chips need an out-of-tree driver, so not at launch |
| AMD GPU | Supported | amdgpu in the kernel; ROCm runs inside the app that needs it |
| NVIDIA GPU | Works with extras | NVIDIA's open kernel modules built and signed in CI; Secure Boot needs the Crustates key enrolled once (MOK). Revisit when the in-kernel Nova driver lands |
| Passthrough | Supported | VFIO for GPUs, NICs, HBAs and USB controllers; the installer's hardware check reports IOMMU groups |
| Accelerators | Works with extras | Coral TPU and Hailo drivers in a signed "hardware extras" layer |
| USB devices | Supported | Zigbee and Z-Wave sticks, UPS units through NUT |
| ARM | Community | Raspberry Pi 5 with Fedora 44 images (microSD boot), apps as shared containers because there is no GICv3-ITS; Ampere and RK3588 boards as upstream support allows |
| Server management | Supported | IPMI and Redfish for power and sensors; TPM 2.0 for automatic unlock of encrypted pools |

- **Signed drivers:** out-of-tree modules (ZFS, NVIDIA, Coral, Hailo) are signed with the Crustates key; the certificate is baked into the image and published, and enrolled once on Secure Boot machines.
- **Hardware extras layer:** optional, signed image layer for non-free or out-of-tree drivers, chosen in the installer or later in Settings › System.
- **Hardware compatibility list:** built from opt-in reports of the installer's hardware check and published on the website.

### Host and operations

- Alerting: notifications by email, ntfy or webhook for disk, pool, backup, update and workload events.
- Metrics, logs and traces through OpenTelemetry, shown in the UI and exportable to any OTLP backend.

### AI assistant

A built-in assistant you can ask about the setup and tell to do things. It is not a separate brain bolted on: it is an MCP client that uses the same API tools as the CLI, the web UI and the external MCP server, so it can only see and do what the API allows.

**What it can do**

- **Answer questions about the environment:** "why is Jellyfin slow?", "which VMs use the most RAM?", "what changed since yesterday?", "is the photos share backed up?". It reads resources, status, metrics, logs, events, alerts, flow logs and the audit log.
- **Troubleshoot:** explain why a resource is degraded, link an alert to the likely cause (full pool, failing disk, OOM kill, expired certificate) and suggest a fix.
- **Take actions:** start, stop and restart VMs and apps, create an app from the catalog, take a snapshot, add a firewall rule or share, run a backup, plan a Molt upgrade. Natural language becomes a normal resource spec you can read before it is applied.
- **Plan:** capacity questions ("can this box take another Windows VM?"), explain a config, draft a Compose import.
- **Digest (opt-in):** a daily or weekly summary of alerts, updates and backup status, sent through the normal alert channels.

**Models: bring your own**

- **Local first:** a model on the box itself (Ollama or llama.cpp, deployed as a catalog App, with GPU passthrough if present) or on another machine on the LAN. This is the privacy default; nothing leaves the network.
- **Remote, opt-in:** any OpenAI-compatible endpoint or the Anthropic API with your own key, stored as a platform secret.
- The UI always shows which provider and model answered, and whether data left the network. Nothing is configured out of the box; the assistant stays off until you pick a provider.

**Grounding**

- Live state through the API tools (never a stale copy), plus retrieval over the Crustates docs, ADRs and your own resource specs, embedded locally.
- Answers cite the resources, log lines or docs they came from, with links into the UI.

**Where it lives**

- Web UI: a chat drawer on every page that knows what you are looking at ("this VM").
- CLI and TUI: `crust ask "..."` and an assistant pane in `crust tui`.
- External agents (Claude Code, Claude Desktop and others) use the same tools through the MCP server, with the same permissions.

**Safety**

- The assistant acts as you, with your role and never more; a per-role allowlist decides which tools it may call at all.
- Read-only by default. Every change is first shown as a plan: the resource diff, a dry run through the reconciler, and what it affects. Nothing is applied without your confirmation.
- Destructive actions (delete, wipe, pool or disk changes, firewall rules that could lock you out) need you to type the resource name, as in the UI.
- Every action lands in the audit log tagged "via assistant" with the prompt that led to it; risky changes take a snapshot first so they can be undone.
- Logs, app output, file names and web content are untrusted input: text found there can never trigger an action on its own (prompt-injection defence). No autonomous actions; the digest only reports.
- Rate limits and a token budget per user, so a looping model cannot hammer the API or a paid provider.

## Tech stack

| Layer | Choice | Language | Notes |
| --- | --- | --- | --- |
| OS image | bootc, composefs-rs | Rust | Fedora base, pinned kernel, ZFS built in CI |
| Init and services | systemd (Varlink where possible) | C | vmspawn for confidential VMs later |
| Control plane API | axum, tower, tokio, utoipa (OpenAPI) | Rust | REST + watch streams; Scalar API reference (utoipa-scalar) served at /docs on every node |
| Node agent | tonic (gRPC), mTLS | Rust | One per host |
| State store | SQLite (sqlx), openraft later | Rust | Single node first |
| VMs and microVMs | QEMU for full VMs, Cloud Hypervisor for microVMs and headless VMs | Rust / C | QEMU microvm as fallback on GICv2 boards |
| Containers | containerd, Kata runtime-rs, crun | Go / Rust / C | runwasi later |
| Storage | ZFS and Btrfs, Samba, NFS, Garage, rustic | Mixed | bcachefs opt-in |
| Networking | netlink, nftables, Aya (eBPF), WireGuard, iroh | Rust / C | NetBird, Tailscale as access plugins; Caddy for ingress and TLS |
| Plugins (Barnacle) | Wasmtime, WIT components | Rust | WASI 0.2 baseline, 0.3 when runtimes settle |
| Observability | OpenTelemetry, eBPF flow data | Rust | Export to any OTLP backend |
| Web UI | TanStack Router SPA, Bun, Vite+, `@crustates/ui` | TypeScript | Neobrutalist; English first, built for translation |
| CLI and TUI | clap, ratatui | Rust | `crust`, `crust tui` |
| Integrations | MCP server, OpenTofu provider | Rust / Go | Clients of the same API |
| AI assistant | rmcp (MCP client), OpenAI-compatible and Anthropic APIs, Ollama or llama.cpp, sqlite-vec | Rust | Local model by default; local embeddings for retrieval over docs and specs |
| GitOps and builds (Spawn) | gitoxide (gix), SOPS + age, BuildKit, Railpack, zot registry | Rust / Go | Pull-based; GitHub App, Forgejo and GitLab webhooks relayed so no port forwarding |

## Security model

Secure by default means a fresh install needs no hardening: the OS is read-only, apps get their own kernel, and the API is the only way in.

- **Boot chain:** Secure Boot with a signed UKI and a Crustates signing key that also signs the ZFS module, TPM measured boot, composefs with fs-verity for the root filesystem.
- **OS:** read-only, image-based, SELinux enforcing, no SSH by default.
- **Workloads:** each VM and microVM runs as its own user with a seccomp filter and its own cgroup, like Firecracker's jailer. Apps default to microVM isolation.
- **API:** built-in accounts with passkeys or TOTP, optional OIDC login, role-based access, mTLS between control plane and agents, audit log of every change.
- **Supply chain:** OS images and platform images signed with cosign (Sigstore); SBOMs and scanning with Trivy; secret scanning with gitleaks.
- **Network:** nftables protects the host; eBPF policy isolates workloads; remote access only through an access provider.
- **AI assistant:** acts with the user's own role, read-only by default, plan-and-confirm for every change, typed confirmation for destructive ones, audit entries tagged "via assistant", and logs and app data treated as untrusted so they cannot steer it (prompt-injection defence).
- **GitOps (Spawn):** pull-based, so no inbound access; deploy keys and GitHub App tokens are scoped to single repos; SOPS + age secrets decrypted only on the node; Git changes pass the same role checks and plans, tagged "via git" with the commit in the audit log.

## Milestones

Each milestone ends with something usable on one real machine; no dates yet.

1. **M0 Foundations:** fresh repo (old PaaS code removed), ADRs 0001–0012, Molt bootc image that boots on amd64 and arm64, CI with VM-in-CI integration tests, signing (including out-of-tree kernel modules), installer image with first-run wizard and hardware check, image registry with stable and beta channels.
2. **M1 First VM:** control plane + agent on one node, VM resource on QEMU with KVM including Windows guest support, `crust tide vm create`, basic bridge networking, OpenAPI spec with the Scalar reference at /docs, VM consoles, cloud-init and templates, an ISO library, DHCP and IPAM, the local recovery console.
3. **M2 Apps:** App resource with Cloud Hypervisor microVMs by default and `isolation: shared` containers, Compose import, image pulls, app logs and shell, secrets, Caddy reverse proxy with the three certificate modes and local names under crust.internal plus mDNS, first app catalog including PostgreSQL, app update detection, Spawn Git sources and push-to-deploy builds (Dockerfile and Railpack).
4. **M3 Storage:** Reef pools (ZFS and Btrfs), SMB and NFS shares, snapshots, scheduled VM and app backups with restore (rustic), platform state backup, SMART monitoring and scrubs, encryption at rest.
5. **M4 Web UI and access:** TanStack dashboard, `crust tui`, built-in accounts (passkeys, TOTP), optional OIDC, roles, NetBird access provider, Molt upgrades from the UI, alerting (email, ntfy, webhook), USB, disk and GPU passthrough, hardware extras layer (NVIDIA, Coral, Hailo), read-only AI assistant (chat drawer and crust ask) with local or remote models.
6. **M5 Current eBPF:** per-workload policy, flow logs and metrics with Aya, OpenTelemetry export.
7. **M6 Cast and Barnacle:** multi-node with Raft, iroh across sites, Wasm plugin system, MCP server and OpenTofu provider, assistant actions with plan-and-confirm, TUI assistant pane, opt-in digest, Spawn PR plans through a GitHub App, preview apps and image update PRs.

## Later

These come after M6, in rough priority order.

1. **Import tools:** bring VMs over from Proxmox, ESXi (OVA) and Unraid.
2. **High availability for Cast:** restart VMs on another node when one fails, live migration, fencing.
3. **UPS integration:** clean shutdown on power loss via NUT.
4. **Power features:** disk spin-down and Wake-on-LAN.
5. **Resource controls:** quotas, CPU pinning, hugepages, memory ballooning and overcommit.
6. **Advanced compute:** confidential VMs (AMD SEV-SNP, Intel TDX), sched\_ext profiles, Wasm as a workload type.
7. **Experimental platform features:** Live Update Orchestrator, bcachefs, XDP fast path.
8. **Firecracker backend for ephemeral jobs:** an optional microVM engine for short-lived, untrusted work such as CI runners, scheduled scripts and AI agent sandboxes, where fast startup matters and shared volumes or GPUs don't. Use its default MMIO transport until virtio-PCI has matured, and drive it directly, since Kata only supports Firecracker through its deprecated Go runtime.
9. **Database resource:** managed PostgreSQL, then MariaDB and Valkey, with users, point-in-time backups to Garage and connection strings injected into apps.

## Way of working and repo

The Crustates way of working carries over unchanged.

- **Repos:** `crustates/crustates` monorepo (started fresh), `crustates/brand`, `crustates/.github`; website and docs later.
- **Start from scratch:** the old PaaS code is gone, so the API setup, auth, agent gRPC, TUI shell, MCP server, xtask and CI are built new in M0 and M1.
- **Planning:** `docs/plan.md` as the master plan with milestones, `CLAUDE.md` for coding agents, numbered MADR ADRs in `docs/adr/`.
- **Build:** `cargo xtask` for Rust, Bun + Vite+ for the web UI.
- **Git:** `development` and `main`, squash merges into development, release-plz for versions, Conventional Commits.
- **Hygiene:** Renovate, Trivy, gitleaks, contract tests for every trait.
- **Testing:** unit and contract tests, VM-in-CI integration tests with nested virtualisation, a small real hardware lab (amd64 and arm64), Playwright for the UI.
- **Versioning and upgrades:** semver releases; a versioned API (`/v1`) with written deprecation rules; each release supported until the next one plus a grace period; upgrades always work from the previous release.
- **Telemetry:** none by default; anything collected is opt-in and documented.
- **Community docs:** docs site, CONTRIBUTING.md, code of conduct, SECURITY.md with private vulnerability reporting.

**First ADRs**

- [ ] 0001 Reposition Crustates as a self-hosted homelab platform, with the names in this plan
- [ ] 0002 Base OS: Fedora bootc with ZFS and Btrfs
- [ ] 0003 No Kubernetes underneath; Kubernetes-shaped API instead
- [ ] 0004 Two compute resources (VM and App) with microVM isolation by default
- [ ] 0005 Licence: AGPLv3 platform, Apache-2.0 SDKs and plugin interfaces, DCO instead of a CLA
- [ ] 0006 Full VMs on QEMU, microVMs on Cloud Hypervisor
- [ ] 0007 Ingress: Caddy with three certificate modes
- [ ] 0008 Default DNS zone `crust.internal` plus mDNS
- [ ] 0009 Built-in accounts with passkeys, OIDC optional
- [ ] 0010 Platform state in SQLite (Raft-replicated later), not PostgreSQL
- [ ] 0011 AI assistant as an MCP client of the public API: bring-your-own model, local by default, plan-and-confirm for actions
- [ ] 0012 Spawn: pull-based GitOps over the public API plus push-to-deploy builds with BuildKit and Railpack

## Licence and base OS

Crustates uses AGPLv3 for the platform, Apache-2.0 for everything others build against, and a DCO sign-off instead of a CLA. The base OS is Fedora bootc, so ZFS and Btrfs are both supported.

**What comparable products do**

| Product | Licence | Notes |
| --- | --- | --- |
| [Proxmox VE](https://proxmoxr.com/blog/proxmox-license-explained) | AGPLv3 | Free for any use, including commercial; revenue from subscriptions; contributors sign a [CLA](https://forum.proxmox.com/goto/post?id=488799) |
| [TrueNAS Community Edition](https://en.wikipedia.org/wiki/TrueNAS) | GPLv3 / LGPLv3 plus proprietary parts | Open core |
| [Incus](https://stgraber.org/page/10/) | Apache-2.0, no CLA | Forked from LXD; stayed Apache when LXD switched |
| [LXD](https://discuss.linuxcontainers.org/t/lxd-has-been-re-licensed-and-is-now-under-a-cla/18454) | AGPLv3 + Apache-2.0 mix, Canonical CLA | Relicensed in 2023; contributors moved to Incus |
| [NetBird](https://wz-it.com/en/expertises/netbird/tailscale-alternative/) | BSD-3 client, AGPLv3 server | The same split proposed here |
| [Dockhand](https://alternativeto.net/software/dockhand/about) | BSL 1.1 | Source-available, becomes Apache-2.0 in 2029 |
| [WolfStack](https://github.com/intelligentwolf/WolfStack) | PolyForm Noncommercial (since v22.10) | Source-available, business use paid |

**Why this licence**

- AGPLv3 is what the closest peer, Proxmox, uses and builds a business on. Users can run it anywhere for free; obligations only start when someone modifies it and distributes or hosts it.
- It stops a cloud provider from selling hosted Crustates with private changes.
- Apache-2.0 for the API spec, client SDKs, plugin interfaces (WIT) and the OpenTofu provider, so plugins and integrations can use any licence.
- DCO instead of a CLA: LXD's switch to AGPL plus a CLA pushed contributors to Incus. Trade-off: relicensing later needs every contributor's consent.
- Not source-available (BSL, PolyForm, FSL): that would break the fully open source principle and put off homelab users and distributions.
- Brand assets are CC BY 4.0 with a trademark policy (TRADEMARK.md in crustates/brand).

**Why Fedora for ZFS and Btrfs**

- CentOS Stream 10 does not ship Btrfs by default: Red Hat is [enabling it only as a module in `kernel-modules-internal`](https://meetbot-raw.fedoraproject.org/meeting-2_matrix_fedoraproject-org/2026-02-11/centos-hyperscale-sig.2026-02-11-16.08.summary.md), which is not installed by default.
- Fedora has Btrfs in its kernel. ZFS is out-of-tree, but [OpenZFS 2.4.4 supports kernels up to Linux 7.2](https://phoronix.com/news/OpenZFS-2.4.4-Released), so the lag is small.
- An image-based OS removes the classic DKMS breakage: the ZFS module is built at image build time, and a kernel it can't build against never ships. [uCore](https://github.com/Myself4626/ucore) does exactly this on Fedora bootc, pins kernels it doesn't trust, and offers an [LTS-kernel stream](https://docsearch.algolia.com/mcp/docs/repo/ublue-os/ucore) for servers.
- With Secure Boot, the self-built ZFS module must be signed with a Crustates key enrolled on first install.
- ZFS is CDDL-licensed; shipping its module in an image is common practice (uCore does it) but debated legally. The AGPL userland is not affected.

## Open decisions and risks

| Item | Type | Notes |
| --- | --- | --- |
| Licence | Decided | AGPLv3 platform, Apache-2.0 SDKs and plugin interfaces, DCO (see above) |
| Base OS | Decided | Fedora bootc, Fedora kernel pinned per release, ZFS built in CI, Btrfs in-kernel |
| Ingress proxy | Decided | Caddy: automatic TLS, built-in local CA, admin API. Revisit River (Rust, on Pingora) if its paused development resumes |
| Full-VM engine | Recommended | QEMU with KVM: display, USB, Windows and GPU support that Cloud Hypervisor lacks |
| MicroVM engine | Recommended | Kata Containers runtime-rs (Rust, default since Kata 4.0) on Cloud Hypervisor; QEMU microvm or shared isolation on GICv2 boards |
| Container manager and runtime | Recommended | containerd as manager (embeddable API, Kata and runwasi shims, ZFS and Btrfs snapshotters; CRI-O needs Kubernetes, Podman overlaps with the agent); crun as OCI runtime, youki once proven |
| Raft library | Recommended | openraft, with SQLite as storage underneath |
| Kernel source | Recommended | Fedora's kernel pinned per release first; a separately built LTS kernel later |
| App catalog format | Recommended | Compose-based format of our own, with importers for existing catalogs |
| Accounts | Recommended | Built-in users with passkeys or TOTP, OIDC optional |
| Assistant model default | Recommended | Off until chosen; the setup suggests a local model sized to the hardware, remote providers opt-in with your own key |
| Image hosting | Decision | Which registry hosts Molt and platform images (for example GitHub Container Registry) |
| UI languages | Decision | English only, or English and Dutch from the start |
| Current's accent colour | Decision | Sea-teal `#0E8A7E` darkened to #0B7A70 (passes WCAG AA); Barnacle takes violet #A77BFF |
| Sustainability model | Decision | Sponsorships, paid support, or none |
| Hardware reports | Decision | Opt-in upload of installer hardware-check results to build the compatibility list: what is collected and where it is stored |
| Spawn prune default | Decision | Resources removed from Git: delete automatically, or only warn (recommended: warn until the user opts in) |
| Assistant name | Decision | Plain "Assistant", or a crab name such as Claw or Pincer |
| ZFS kernel lag | Risk | A new Fedora kernel can land before OpenZFS supports it; mitigated by pinning and the CI gate |
| Scope creep | Risk | Proxmox, TrueNAS and Portainer in one is huge; hold each milestone to one working machine |
| Raspberry Pi support | Risk | No GICv3-ITS, so no Cloud Hypervisor microVMs; bootc boot support on the Pi is unverified |
| MicroVM overhead | Risk | Tens of MB RAM per app; `isolation: shared` is the escape hatch |
| Experimental features | Risk | bcachefs, Live Update Orchestrator and WASI 0.3 stay opt-in until stable |
| Solo maintenance | Risk | Lean on reused engines; keep Rust code to control plane, agent and drivers |
| Assistant mistakes | Risk | Models can be confidently wrong, and small local models more so; mitigated by plan-and-confirm, dry runs, snapshots and the audit log |
