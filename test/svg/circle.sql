-- A parametric curve drawn in parameter order, on a numeric axis with the default mesh.
SELECT chart().draw_series(line_series(cos(t), sin(t), order_by := t)).to_svg()
FROM (SELECT i / 20.0 AS t FROM range(126) r(i));
