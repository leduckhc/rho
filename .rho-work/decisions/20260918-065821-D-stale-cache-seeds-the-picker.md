# D-stale-cache-seeds-the-picker — show stale cached models while a fresh list loads

**Decision:** the model picker seeds itself from any cached model list while it waits
for a fresh listing. Cached rows are marked stale. A fresh listing that confirms an id
clears its stale marker. A provider catalog that has no cache returns `None` from
`peek_cached_models`.

**Reason:** a user should never see an empty picker on first open. The cache is
advisory, so showing it as stale while refreshing keeps the picker useful without
pretending the list is current.

**Rule:** `ModelCatalog::peek_cached_models` returns cached models without a network
call. The default is `None`, because most catalogs have no on-disk cache.
`CatalogCache` overrides it. The TUI owns the stale marker and the renderer shows it.
