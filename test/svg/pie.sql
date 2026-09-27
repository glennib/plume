-- A pie: one slice per row, largest first (order_by), starting at twelve o'clock, with
-- percentages inside the slices and larger labels.
SELECT pie(n, section, order_by := -n)
         .start_angle(-90)
         .label_style(14)
         .percentages(font('sans-serif', 12).color('white'))
         .to_svg()
FROM (SELECT section, count(*) AS n FROM articles GROUP BY section);
