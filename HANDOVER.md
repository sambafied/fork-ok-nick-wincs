# Fresh-session prompt: AEM desktop bridge

Develop the Rust AEM Desktop Bridge using the approved specifications, with Windows CFAPI and the maintained wincs fork as the primary filesystem backend. Support on-premises AEM and AEM as a Cloud Service. Deliver the original Adobe desktop application's workflows plus native Explorer folder access.

Use the existing repository clones, with paths relative to their shared parent:

- Product: `./desktop-client-adobe`
- Maintained wrapper: `./fork-ok-nick-wincs`

Work in these existing clones. Locate their shared parent from the current workspace, inspect Git status and ownership, and fetch current remote branches without disturbing local work. Inspect code, open issues, repository instructions and normative specifications to establish current status. This prompt requires no previous chat, existing issue worktree, cached build, retained test fixture or running AEM instance.

Read the product's AGENTS.md, indexed governance, specification, architecture, compatibility contract, docs/cfapi-primary.md and docs/dam-reads.md. Declare Development and bind work to tracked requirements and issues.

Known delivered baseline to verify: product PR 41 implements local AEM login, paginated DAM browsing and verified original downloads through the CLI. Fork PR 19 adds observable fallible disconnect. Historical test evidence is in the repositories; rerun appropriate checks for the actual candidate and environment. The desktop UI and complete folder sync experience remain outstanding.

Begin with product issue 40: connect DAM reads to an owned CFAPI sync root, populate folders and hydrate original assets when opened through the filesystem. Review the current fork pin and adopt a verified immutable merged revision when needed. Address remaining wrapper safety issues required by the app's runtime paths, including issue 7's failure and recovery matrix.

Implement identity-bound root ownership, safe paths, callback lifetimes, epoch and cancellation checks, observable connect/stop/unregister and retained bytes on failure. Unregister only after confirmed disconnect completion. Preserve unrelated and locally modified files. Verify native read-only enforcement before offering that mode. Avoid borrowed pointers across asynchronous work, panics across FFI and state locks over network or unbounded native operations.

Qualify real Windows folder enumeration, separate-process file opening and content hashes, logout, provider loss, reconnect and retained data. Establish a fresh isolated test environment and generated fixtures; discover available AEM test endpoints and use protected credential input. Request missing external access only when required while progressing independent work. Keep local AEM, SaaS, wrapper, creative-app and package acceptance distinct.

Then work through the approved UI and sync backlog: browsing and connection UI, local edits, uploads/publication, conflicts, offline/restart recovery, pin/free-space controls, creative-app workflows and Windows package/Shell integration. Use original-app workflow qualification and approved specs as acceptance criteria. Complete bounded deliveries without claiming full parity prematurely.

The user authorizes implementation, testing, signed commits, immediate pushes, PR creation and eligible merges. Follow repository and host worktree policies, preserve other workers and user data, and clean owned disposable output. Land product work through PRs into develop and wrapper work through PRs into main. Update docs, evidence and issues as behavior lands. Release deployment and production/SaaS mutations require separate authorization.
