This crate provides a generic type system and database layer.

We want to extend this system with a postgres adapter, that maps classes from the 
type system to postgres tables.

This should include a discovery mechanism that discovers and populates the 
catalog schema (see crates/db_core) with types derived from the postgres table
structure.

Since this system expects a globally unique ID to be available, we synthesize
the global id property through concatenating the table name + '-' + the native
db unique key, which also has to be determined from the schema.

note, as per the Makefile, there already is a local postgres server running.
But tests that cover postgres should rely on a POSTGRES_URI env var to be present,
and use that. 

We can use computed class attributes for this. see struct ClassAttribute, field
computed. We can use a concat + stringify expression with self. , etc.
The postgres backend should only allow entities with a class, class-less entities
are forbidden.

Thoroughly check the code, then craft a detailed step by step implementation plan,
and final acceptance criteria, including testing strategy.
Write it to ./plan.md, next to this task.md
