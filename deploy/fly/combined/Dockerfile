# syntax=docker/dockerfile:1
# Build the existing node base and hub layer first (see README.md).
ARG WKS_BASE=workspacer-hub:dev
FROM ${WKS_BASE}
COPY deploy/fly/combined/entrypoint.sh /usr/local/lib/wks-combined/entrypoint.sh
RUN chmod 0755 /usr/local/lib/wks-combined/entrypoint.sh
RUN /usr/local/lib/wks/verify-image.sh
EXPOSE 7895
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/lib/wks-combined/entrypoint.sh"]
