-- area_series with a translucent fill, a border and a baseline of 5, and a second area without a
-- border drawn over it; the legend glyphs are filled rectangles.
SELECT chart()
         .caption('area_series', 20)
         .draw_series(area_series(i, 20 + 10 * sin(i / 5))
                        .style(mix('blue_400', 0.3)).border_style('blue_800', 2).baseline(5).label('level'))
         .draw_series(area_series(i, 15 + 5 * cos(i / 3)).style(mix('orange', 0.5)).baseline(5).label('inflow'))
         .configure_series_labels().position('upper_right').border_style('black').background_style('white').draw()
         .to_svg()
FROM range(41) r(i);
