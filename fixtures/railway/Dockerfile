FROM node:22-slim AS build
WORKDIR /app
COPY package.json ./
RUN npm install
COPY . .
RUN npx rex build

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/node_modules/@limlabs/rex-linux-x64/bin/rex /usr/local/bin/rex
WORKDIR /app
COPY --from=build /app/.rex .rex
EXPOSE 3000
CMD ["rex", "start", "--port", "3000"]
