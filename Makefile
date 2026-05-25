

# Run a postgres server with docker.
# To connect, use user postgres, password postgres
run-postgres:
	docker run --name postgres -e POSTGRES_PASSWORD=postgres -p 5432:5432 -d postgres:alpine
