ARG REX_VERSION=latest

# Build stage: uses the app-build target which includes `rex build`
FROM ghcr.io/limlabs/rex:${REX_VERSION}-build AS build

WORKDIR /app
COPY pages pages
RUN rex build

# Runtime stage: slim image with only the runtime binary
FROM ghcr.io/limlabs/rex:${REX_VERSION}

WORKDIR /app
COPY --from=build /app/.rex /app/.rex
COPY --from=build /app/pages /app/pages

EXPOSE 3000
