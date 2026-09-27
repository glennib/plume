-- The CLI renders custom-type values through their VARCHAR cast, in every output mode.
.mode box
SELECT duckers_version() LIKE 'v%' AS versioned,
       __duckers_envelope('CHART', 'abc'::BLOB)::CHART AS chart,
       __duckers_envelope('SERIES', ''::BLOB)::SERIES AS series;
.mode csv
SELECT __duckers_envelope('MESH', 'ab'::BLOB)::MESH AS mesh, NULL::FONT AS font;
.mode json
SELECT __duckers_envelope('SERIES_LABELS', 'x'::BLOB)::SERIES_LABELS AS labels;
