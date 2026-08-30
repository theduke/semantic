we want the web server of this project to be a multi-tenant web server that can open different dbs and sessions and serve different users.

Auth (with tokens) or just db selection in queries / connections would determine which auth scope you connect to. an auth scope could have access to different dbs.
There would also be admin accounts that can access all known dbs.

we would probably need a special separate root auth db. 
And an account system, with a token system attached.
Later we'd also want to add sophisticated permission systems , object verb subject style, including dynamic / scoped subjects and hierarchy, essentially a full blown permission system that should be in it's own separate crate (core logic), but integrated well into the whole system.

research, and draft a comprehensive implementation plan

write a full plan to ./docs/plans/<date>-multitenant-system/plan.md
also write this prompt (without the this line) to the same dir under /prompt.md
