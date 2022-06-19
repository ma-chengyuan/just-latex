#!/usr/bin/bash

for file in demo fwht styles 
do
    pandoc $file.md --filter ../target/debug/just-latex -o $file.html
done
