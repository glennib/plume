-- error_bar_vertical(x, min, avg, max) with filled dots over a line through the averages; the y axis
-- spans the minima and maxima.
SELECT chart()
         .caption('error_bar_vertical', 20)
         .draw_series(line_series(i, i * i / 10).style('grey'))
         .draw_series(error_bar_vertical(i, i * i / 10 - 1 - i % 3, i * i / 10, i * i / 10 + 2)
                        .style('red').width(8).filled().label('measured'))
         .configure_series_labels().position('upper_left').draw()
         .to_svg()
FROM range(11) r(i);
