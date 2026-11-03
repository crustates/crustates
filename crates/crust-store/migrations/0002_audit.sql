-- The audit log: one row per change made through the API, never updated or
-- deleted by the platform. `id` keeps the order of entries with equal times.
CREATE TABLE audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    time TEXT NOT NULL,
    principal TEXT NOT NULL,
    action TEXT NOT NULL,
    target TEXT NOT NULL,
    outcome TEXT NOT NULL
) STRICT;
