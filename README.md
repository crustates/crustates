# Crustates

**Your own cloud, on your own hardware.**

Crustates runs VMs, apps, storage and networking on your homelab as one Rust
platform, with a read-only OS and a declarative API.

**Status:** early: milestone M0 (Foundations). Nothing here is usable yet.

## Product areas

| Area          | What it covers                    |
| ------------- | --------------------------------- |
| **Tide**      | Compute: VMs, microVMs and apps   |
| **Reef**      | Storage                           |
| **Current**   | Networking                        |
| **Molt**      | The read-only OS and its upgrades |
| **Cast**      | Clusters                          |
| **Barnacle**  | Plugins                           |
| **Spawn**     | GitOps                            |
| **Assistant** | The AI assistant                  |

## Links

- Website: [crustates.eu](https://crustates.eu)
- Docs: [docs.crustates.eu](https://docs.crustates.eu)
- Roadmap: [GitHub Projects](https://github.com/orgs/crustates/projects)
- Questions and ideas: [Discussions](https://github.com/crustates/crustates/discussions)
- Contributing: [CONTRIBUTING.md](CONTRIBUTING.md)

## Licence

The platform is licensed under the [GNU AGPL v3](LICENSE). The client SDK in
[`sdk/`](sdk/) and the plugin interfaces in [`wit/`](wit/) are licensed under
Apache-2.0, so clients and plugins can use any licence.
