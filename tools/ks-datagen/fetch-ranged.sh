#!/bin/sh
# Докачка выгрузки Tatoeba в кэш ks-datagen по частям: на некоторых сетях соединение с
# downloads.tatoeba.org отдаёт ~16 КБ и замирает, и обычная загрузка (ureq, curl) не доходит.
# Использование: tools/ks-datagen/fetch-ranged.sh rus|eng   (из корня репозитория)
set -eu
l=$1
C=target/ks-datagen-cache
U=https://downloads.tatoeba.org/exports/per_language/$l/${l}_sentences.tsv.bz2
W=$C/${l}.parts
mkdir -p "$W"
size=$(curl -sI "$U" | grep -i '^content-length' | tr -dc 0-9)
chunk=12000
n=$(( (size + chunk - 1) / chunk ))
curl -sI "$U" | grep -i '^last-modified' | sed 's/^[^:]*: //' | tr -d '\r\n' > "$C/${l}_sentences.tsv.bz2.last-modified"
for pass in 1 2 3 4 5 6 7 8; do
  todo=$(i=0; while [ $i -lt $n ]; do
    a=$((i * chunk)); b=$((a + chunk - 1)); [ $b -ge $size ] && b=$((size - 1))
    f=$W/$(printf %06d $i); have=$(wc -c < "$f" 2>/dev/null || echo 0)
    [ "$have" -eq $((b - a + 1)) ] || echo "$i $a $b"; i=$((i + 1)); done)
  [ -z "$todo" ] && break
  echo "проход $pass: осталось $(printf '%s\n' "$todo" | wc -l | tr -d ' ') частей"
  printf '%s\n' "$todo" | xargs -P 32 -L 1 sh -c 'curl -s --max-time 12 -r "$1-$2" -o "'"$W"'/$(printf %06d "$0")" "'"$U"'"' || true
done
cat "$W"/0* > "$C/${l}_sentences.tsv.bz2.part"
bunzip2 -t "$C/${l}_sentences.tsv.bz2.part"
mv "$C/${l}_sentences.tsv.bz2.part" "$C/${l}_sentences.tsv.bz2"
rm -r "$W"
echo "$l: $(wc -c < "$C/${l}_sentences.tsv.bz2") Б"
