-- The CLI renders custom-type values through their VARCHAR cast, in every output mode.
.mode box
SELECT plume_version() LIKE 'v%' AS versioned,
       chart().caption('Oslo').draw_series(line_series(i, i, key := i % 2)) AS chart,
       line_series(i, i) AS series,
       point_series(i, i, key := i % 2) AS keyed
FROM range(10) r(i);
.mode csv
SELECT chart().configure_mesh().x_desc('day') AS mesh, NULL::FONT AS font;
.mode json
SELECT chart().configure_series_labels().position('upper_left') AS labels;
