# Postfix from Alpine, for the same reason Dovecot is: one image that runs on both arm64 and amd64.
#
# The listener on port 25 is taken out and one on 587 put in its place, so the only way into this
# container is a submission conversation from somebody who signs in.
FROM alpine:3.20

RUN apk add --no-cache postfix=3.9.11-r0 \
    && postconf -MX smtp/inet \
    && postconf -M submission/inet="submission inet n - n - - smtpd"

COPY config/postfix.cf /etc/postfix/main.cf

EXPOSE 587
CMD ["postfix", "start-fg"]
