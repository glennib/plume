-- The shim alone is loaded (the Makefile puts the core next to it), so loading the shim has
-- loaded the core, and VISUALIZE works once the grammar is active.
.mode csv
SELECT extension_name, loaded FROM duckdb_extensions() WHERE extension_name LIKE 'plume%' ORDER BY 1;
SET active_grammar_extensions = ['plume_visualize'];
SELECT i FROM range(5) r(i) VISUALIZE chart().draw_series(line_series(i, i * i)) AS chart;
SELECT current_setting('active_grammar_extensions') AS active;
