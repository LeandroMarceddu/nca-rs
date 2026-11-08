# duplicate-holes

Drills (20, 20) twice with the same tool and Z range, then one different hole.
Expected: exactly one `geometry.duplicate_hole` warning on the repeated hole;
the hole at (40, 20) is a near-miss negative.

Golden draft: `geometry.duplicate_hole` is not implemented yet, so the draft
has no diagnostics. The canned-cycle holes still parse into motion records.
