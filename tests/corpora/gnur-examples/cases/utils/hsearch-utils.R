RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: hsearch-utils
### Title: Help Search Utilities
### Aliases: hsearch_db hsearch_db_concepts hsearch_db_keywords
### Keywords: documentation

### ** Examples

db <- hsearch_db()
## Total numbers of documentation objects, aliases, keywords and
## concepts (using the current format):
sapply(db, NROW)
## Can also be obtained from print method:
db
## 10 most frequent concepts:
head(hsearch_db_concepts(), 10)
## 10 most frequent keywords:
head(hsearch_db_keywords(), 10)



