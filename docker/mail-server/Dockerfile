# Dovecot from Alpine rather than the upstream image: that one publishes amd64 only, and half the
# development machines here are arm64, where it dies under emulation.
#
# LMTP is a package of its own on Alpine, and it is what the submission container delivers into.
FROM alpine:3.20

RUN apk add --no-cache dovecot=2.3.21.1-r0 dovecot-lmtpd=2.3.21.1-r0 \
    && adduser -D -u 1000 -h /srv/mail mailuser \
    && mkdir -p /srv/mail \
    && chown mailuser:mailuser /srv/mail

COPY config/dovecot.conf /etc/dovecot/dovecot.conf

# 143 is published to this machine's loopback; 24 and 12345 stay on the network compose makes.
EXPOSE 143 24 12345
CMD ["dovecot", "-F"]
