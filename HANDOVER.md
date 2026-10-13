# Fresh-session prompt: AEM desktop bridge

Develop the Rust AEM Desktop Bridge using the approved specifications, with Windows CFAPI and the maintained wincs fork as the primary filesystem backend. Support on-premises AEM and AEM as a Cloud Service. Deliver the original Adobe desktop application's workflows plus native Explorer folder access.

Use the existing repository clones, with paths relative to their shared parent:

- Product: `./desktop-client-adobe`
- Maintained wrapper: `./fork-ok-nick-wincs`
- Copied local AEM kit: `./_kits/aem`

Work in these existing clones. Locate their shared parent from the current workspace, inspect Git status and ownership, and fetch current remote branches without disturbing local work. Inspect code, open issues, repository instructions and normative specifications to establish current status. This prompt requires no previous chat, existing issue worktree, cached build, retained test fixture or running AEM instance.

Read the product's AGENTS.md, indexed governance, specification, architecture, compatibility contract, docs/cfapi-primary.md and docs/dam-reads.md. Declare Development and bind work to tracked requirements and issues.

For local AEM qualification, inspect `./_kits/aem` and its `start.ps1` launcher. Check whether the intended Author is already running before starting another process. If it is stopped, verify the available JAR, Java compatibility and launcher parameters, then start it headlessly from the shared parent with `& ./_kits/aem/start.ps1`. The launcher's default Author URL is `http://127.0.0.1:4502`; verify readiness, license state and authenticated DAM access separately before testing the bridge. Preserve the copied kit's repository, license, logs and runtime data. Enter credentials through protected client input; do not put them in Git, evidence or command arguments.

Known delivered baseline to verify: product PR 41 implements local AEM login, paginated DAM browsing and verified original downloads through the CLI. Product PR 42 completes bounded issue 40: an owned CFAPI folder with lazy original recalls, private ACLs, lifecycle/status and retained edits. Fork PR 19 adds observable fallible disconnect; PR 24 repairs failed-recall byte ranges. Historical test evidence is in the repositories; rerun appropriate checks for the actual candidate and environment. The desktop UI and complete folder sync experience remain outstanding.

Read docs/cfapi-local-preview.md for current use and qualification. Verify issue 40 is closed and preserve its delivered behavior; select the next ready issue from the approved backlog and dependency order. Remaining filesystem tracking, pin/offline controls, snapshot uploads and desktop UI are tracked under product issues 6/7, 8 and 11. Review the current immutable fork pin before dependency changes. Address remaining wrapper safety issues required by each runtime path, including issue 7's full failure/recovery matrix.

Implement identity-bound root ownership, safe paths, callback lifetimes, epoch and cancellation checks, observable connect/stop/unregister and retained bytes on failure. Unregister only after confirmed disconnect completion. Preserve unrelated and locally modified files. Verify native read-only enforcement before offering that mode. Avoid borrowed pointers across asynchronous work, panics across FFI and state locks over network or unbounded native operations.

Qualify real Windows folder enumeration, separate-process file opening and content hashes, logout, provider loss, reconnect and retained data. Establish a fresh isolated test environment and generated fixtures; discover available AEM test endpoints and use protected credential input. Request missing external access only when required while progressing independent work. Keep local AEM, SaaS, wrapper, creative-app and package acceptance distinct.

Then work through the approved UI and sync backlog: browsing and connection UI, local edits, uploads/publication, conflicts, offline/restart recovery, pin/free-space controls, creative-app workflows and Windows package/Shell integration. Use original-app workflow qualification and approved specs as acceptance criteria. Complete bounded deliveries without claiming full parity prematurely.

The user authorizes implementation, testing, signed commits, immediate pushes, PR creation and eligible merges. Follow repository and host worktree policies, preserve other workers and user data, and clean owned disposable output. Land product work through PRs into develop and wrapper work through PRs into main. Update docs, evidence and issues as behavior lands. Release deployment and production/SaaS mutations require separate authorization.
