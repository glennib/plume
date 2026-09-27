-- A legend with every SERIES_LABELS option: a coordinate position, margin, legend area size,
-- border and background styles, and a FONT with a colour.
SELECT chart()
         .draw_series(list_transform(line_series(day, temp, key := city), lambda s: s.stroke_width(2)))
         .draw_series(point_series(day, temp).marker('cross').size(3).style('black').label('samples'))
         .configure_series_labels()
           .position(20, 10)
           .margin(8)
           .legend_area_size(40)
           .border_style('grey_600', 2)
           .background_style(mix('white', 0.85))
           .label_font(font('serif', 16, 'italic').color('bluegrey_800'))
           .draw()
         .to_svg()
FROM weather
WHERE day < DATE '2024-02-01';
