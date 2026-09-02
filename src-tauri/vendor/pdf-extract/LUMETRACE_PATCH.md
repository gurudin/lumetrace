# Lume Trace patch

This directory contains `pdf-extract` 0.9.0, licensed under MIT.

Lume Trace changes the `gs` graphics-state handler so a PDF that references a
missing `ExtGState` resource does not panic. Missing graphics state affects
rendering properties such as transparency and must not prevent text extraction.
