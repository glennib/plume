-- The four PointSeries markers, one keyed series each (keys sort as the marker names), filled
-- where the element supports it, each in the legend with its own glyph. plotters' Pixel is one
-- pixel, whatever the size.
SELECT chart()
         .caption('marker()', 20)
         .draw_series(list_transform(point_series(x, y, key := m),
             lambda s, n: s.marker(['circle', 'cross', 'pixel', 'triangle'][n])
                           .style(['red', 'green_700', 'black', 'blue_600'][n])
                           .size(5).filled()))
         .configure_series_labels().position('lower_right').border_style('black').background_style('white').draw()
         .to_svg()
FROM (
    SELECT i AS x, j + sin(i / 3) AS y, ['circle', 'cross', 'pixel', 'triangle'][j + 1] AS m
    FROM range(20) r(i), range(4) s(j)
);
