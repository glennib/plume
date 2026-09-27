-- x_monthly() on a date-bucket axis: one band per day, key points on the bands of month starts,
-- labelled with a strftime pattern.
SELECT chart()
         .caption('x_monthly() on date buckets', 20)
         .x_monthly()
         .configure_mesh().x_label_formatter('%b').draw()
         .draw_series(histogram_vertical(DATE '2024-01-01' + i::INTEGER, (i * 37) % 11).margin(0))
         .to_svg(640, 400)
FROM range(100) r(i);
