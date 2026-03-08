ARG REX_VERSION=latest
FROM ghcr.io/limlabs/rex:${REX_VERSION}

WORKDIR /app
COPY pages pages
RUN rex build

EXPOSE 3000
