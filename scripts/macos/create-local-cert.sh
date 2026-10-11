#!/bin/sh
# Один раз: создать в связке ключей «вход» самоподписанный сертификат подписи кода
# «KeySwitcher Local». Подпись им не меняется между сборками, и права TCC не слетают.
# Скрипт меняет связку ключей пользователя — запускается вручную, осознанно.
# Вручную то же самое: «Связка ключей» → Ассистент сертификации → Создать сертификат…
#   имя «KeySwitcher Local», тип идентичности «Самоподписанный корневой», тип «Подпись кода».
set -eu
NAME="KeySwitcher Local"
if security find-identity -p codesigning | grep -q "\"$NAME\""; then
    echo "сертификат «$NAME» уже есть"
    exit 0
fi
TMP=$(mktemp -d)
cleanup() {
    rm -f "$TMP/cert.cnf" "$TMP/key.pem" "$TMP/cert.pem" "$TMP/cert.p12"
    rmdir "$TMP"
}
trap cleanup EXIT
cat > "$TMP/cert.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NAME
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
CNF
# /usr/bin/openssl (LibreSSL) пишет PKCS#12 в формате, который понимает `security import`.
/usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -config "$TMP/cert.cnf" \
    -keyout "$TMP/key.pem" -out "$TMP/cert.pem"
PASS=keyswitcher-local
/usr/bin/openssl pkcs12 -export -inkey "$TMP/key.pem" -in "$TMP/cert.pem" -name "$NAME" \
    -passout "pass:$PASS" -out "$TMP/cert.p12"
security import "$TMP/cert.p12" -k "$HOME/Library/Keychains/login.keychain-db" -P "$PASS" \
    -T /usr/bin/codesign
echo "готово. Проверка: security find-identity -p codesigning"
echo "сборка: KS_SIGN_IDENTITY=\"$NAME\" scripts/macos/build-app.sh"
