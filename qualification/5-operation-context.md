# Callback operation context

Fork issue #5 and product W39. Carry the original callback operation context through every native success and failure reply. The product adopts an immutable fork revision after source and test review.

## Owned context and compatibility

Copy the callback connection, transfer and request keys as opaque values before callback work begins. Copy optional correlation data into owned fixed storage whose lifetime covers the native call. The original callback pointer must not survive through asynchronous work.

Expose an opaque context identity to product consumers. Preserve zero values and exact equality. A reused tuple is not evidence that an earlier callback has completed. Ambiguous ownership remains unresolved until actual lifecycle evidence establishes it.

Each of the six ticket types owns the same original context. Proxy success, callback error, panic failure and batch placeholder replies use that retained context. Context-aware command execution and failure preserve the original native HRESULT and returned union. Existing public connection/transfer pair commands keep their documented default request-key behavior for compatibility.

## Cancellation ranges

Provide checked range readers for required fetch, optional fetch and cancel data. Validate signed offsets and lengths, overflow and EOF semantics against the actual file size. Keep old APIs available while callers migrate.

Product cancellation matches the actual original operation identity and maintains needed byte ranges. A cancel range can cover only part of the outstanding transfer. Cancelling one subset must preserve the remaining request. Preserve native transfer alignment and EOF rules, and record full hydration only after actual complete byte/hash and materialization evidence.

## Validation and landing

Pure tests exercise exact native operation construction through existing executor seams, all ticket and failure reply paths, optional correlation ownership, zero and reused tuples, checked range endpoints, overflow and EOF. Native callback, cancellation ordering, ambiguous reuse and product hydration require independently admitted Windows receipts.

Land signed commits through sambafied branches and PRs into fork main. Immediately push each signed commit. Keep the current product pin until the complete fork change passes its gates. The product then reviews and adopts the exact immutable revision, including every callback, failure and consumer lifetime.
