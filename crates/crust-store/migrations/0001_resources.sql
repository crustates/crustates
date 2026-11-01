-- Every resource of every kind, stored as one JSON document. `generation` is
-- copied out of the document so updates can compare it in SQL.
CREATE TABLE resources (
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    generation INTEGER NOT NULL,
    body TEXT NOT NULL,
    PRIMARY KEY (kind, name)
) STRICT;
