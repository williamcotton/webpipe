/* r.c – Web Pipe “R” middleware  (link with libR)  */
#define R_NO_REMAP
#include <R.h>
#include <Rinternals.h>
#include <Rembedded.h>
#include <Rinterface.h>
#include <R_ext/Parse.h>

#include <jansson.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <unistd.h>
#include <signal.h>

/* --------- compile‑time options set by your Makefile (see embed_r) ----- */
#ifndef DEFAULT_R_HOME      /* gets defined on macOS build */
#define DEFAULT_R_HOME NULL
#endif

/* ---------- middleware interface -------------------------------------- */
typedef void* (*arena_alloc_func)(void*, size_t);
typedef void  (*arena_free_func)(void*);

json_t *middleware_execute(json_t *input,          /* request JSON        */
                           void   *arena,          /* bump‑allocator       */
                           arena_alloc_func alloc, /* arena malloc         */
                           arena_free_func  free_, /* (unused)             */
                           const char *filter,     /* R code               */
                           json_t *config,         /* per‑step config      */
                           char **contentType,     /* out param            */
                           json_t *variables);     /* (unused)             */

int  middleware_init(json_t *config);              /* server start‑up      */
void __attribute__((destructor)) r_middleware_fini(void);

/* ---------- global interpreter state ---------------------------------- */
static pthread_mutex_t r_lock      = PTHREAD_MUTEX_INITIALIZER;
static int             r_running   = 0;

/* Store original signal handlers to restore after R init */
static struct sigaction original_sigint_handler;
static struct sigaction original_sigterm_handler;

/* very small N‑way hash for parsed expressions ------------------------- */
#define R_CACHE_SZ  64
typedef struct r_cache_entry {
    char *key;          /* filter string            */
    SEXP  expr;         /* parsed expression        */
    struct r_cache_entry *next;
} r_cache_entry;
static r_cache_entry *r_cache[R_CACHE_SZ] = {0};

static unsigned hash_key(const char *s)
{
    unsigned h = 5381;
    while (*s) h = ((h<<5)+h) + (unsigned char)*s++;
    return h % R_CACHE_SZ;
}

/* helper: parse or retrieve cached expression ------------------------- */
static SEXP cached_parse(const char *src)
{
    unsigned h = hash_key(src);
    for (r_cache_entry *e = r_cache[h]; e; e = e->next)
        if (strcmp(e->key, src) == 0) return e->expr;

    /* not cached – parse now */
    ParseStatus status;
    SEXP s   = PROTECT(Rf_mkString(src));
    SEXP vec = PROTECT(R_ParseVector(s, -1, &status, R_NilValue));
    UNPROTECT(1);                       /* drop string */
    if (status != PARSE_OK || LENGTH(vec) < 1) {
        UNPROTECT(1);
        return R_NilValue;
    }
    
    /* For multi-line expressions, we need to store the entire vector, not just first expr */
    SEXP expr = vec;  /* Store the entire expression vector */
    R_PreserveObject(expr);             /* keep across GC */
    UNPROTECT(1);

    r_cache_entry *n = malloc(sizeof(*n));
    n->key  = strdup(src);
    n->expr = expr;
    n->next = r_cache[h];
    r_cache[h] = n;
    return expr;
}

/* ---------- JSON <‑‑> R trivial bridge (scalars + lists) -------------- */
static SEXP json_to_sexp(json_t *j);
static json_t *sexp_to_json(SEXP x);

static SEXP json_to_sexp(json_t *j)
{
    if (json_is_null(j))     return R_NilValue;
    if (json_is_boolean(j))  return Rf_ScalarLogical(json_boolean_value(j));
    if (json_is_integer(j)) {
        json_int_t int_val = json_integer_value(j);
        return Rf_ScalarReal((double)int_val);
    }
    if (json_is_real(j))     return Rf_ScalarReal(json_real_value(j));
    if (json_is_string(j))   return Rf_mkString(json_string_value(j));

    if (json_is_array(j)) {
        size_t n = json_array_size(j);
        
        /* Check if it's a homogeneous numeric array */
        int all_numbers = 1;
        for (size_t i = 0; i < n; ++i) {
            json_t *elem = json_array_get(j, i);
            if (!json_is_number(elem)) {
                all_numbers = 0;
                break;
            }
        }
        
        if (all_numbers && n > 0) {
            /* Create numeric vector */
            SEXP vec = PROTECT(Rf_allocVector(REALSXP, (R_xlen_t)n));
            for (size_t i = 0; i < n; ++i) {
                json_t *elem = json_array_get(j, i);
                double val;
                if (json_is_integer(elem)) {
                    json_int_t int_val = json_integer_value(elem);
                    val = (double)int_val;
                } else {
                    val = json_real_value(elem);
                }
                REAL(vec)[(R_xlen_t)i] = val;
            }
            UNPROTECT(1);
            return vec;
        } else {
            /* Mixed types or empty - use generic list */
            SEXP vec = PROTECT(Rf_allocVector(VECSXP, (R_xlen_t)n));
            for (size_t i = 0; i < n; ++i)
                SET_VECTOR_ELT(vec, (R_xlen_t)i, json_to_sexp(json_array_get(j,i)));
            UNPROTECT(1);
            return vec;
        }
    }
    if (json_is_object(j)) {
        size_t len = json_object_size(j);
        SEXP names = PROTECT(Rf_allocVector(STRSXP, (R_xlen_t)len));
        SEXP vals  = PROTECT(Rf_allocVector(VECSXP, (R_xlen_t)len));
        size_t idx = 0;
        const char *k;
        json_t     *v;
        json_object_foreach(j, k, v) {
            SET_STRING_ELT(names, (R_xlen_t)idx, Rf_mkChar(k));
            SET_VECTOR_ELT(vals, (R_xlen_t)idx, json_to_sexp(v));
            ++idx;
        }
        Rf_setAttrib(vals, R_NamesSymbol, names);
        UNPROTECT(2);
        return vals;
    }
    return R_NilValue; /* fallback */
}

static json_t *sexp_to_json(SEXP x)
{
    if (x == R_NilValue) {
        return json_null();
    }
    
    R_xlen_t len = XLENGTH(x);
    
    switch (TYPEOF(x)) {
    case NILSXP:         
        return json_null();
        
    case LGLSXP:         
        if (len == 1) {
            return json_boolean(LOGICAL_ELT(x,0));
        } else {
            json_t *arr = json_array();
            for (R_xlen_t i = 0; i < len; i++)
                json_array_append_new(arr, json_boolean(LOGICAL_ELT(x,i)));
            return arr;
        }
        
    case INTSXP:         
        if (len == 1) {
            return json_integer(INTEGER_ELT(x,0));
        } else {
            json_t *arr = json_array();
            for (R_xlen_t i = 0; i < len; i++)
                json_array_append_new(arr, json_integer(INTEGER_ELT(x,i)));
            return arr;
        }
        
    case REALSXP:        
        if (len == 1) {
            return json_real(REAL_ELT(x,0));
        } else {
            json_t *arr = json_array();
            for (R_xlen_t i = 0; i < len; i++)
                json_array_append_new(arr, json_real(REAL_ELT(x,i)));
            return arr;
        }
        
    case STRSXP:         
        if (len == 1) {
            return json_string(CHAR(STRING_ELT(x,0)));
        } else {
            json_t *arr = json_array();
            for (R_xlen_t i = 0; i < len; i++)
                json_array_append_new(arr, json_string(CHAR(STRING_ELT(x,i))));
            return arr;
        }
        
    case VECSXP: {
        SEXP names = Rf_getAttrib(x, R_NamesSymbol);
        
        if (Rf_isNull(names)) {
            /* treat as JSON array */
            json_t *arr = json_array();
            for (R_xlen_t i = 0; i < len; i++)
                json_array_append_new(arr, sexp_to_json(VECTOR_ELT(x,i)));
            return arr;
        } else {
            /* treat as named object */
            json_t *obj = json_object();
            for (R_xlen_t i = 0; i < len; i++) {
                const char *name = CHAR(STRING_ELT(names,i));
                json_object_set_new(obj,
                                    name,
                                    sexp_to_json(VECTOR_ELT(x,i)));
            }
            return obj;
        }
    }
    
    /* Handle other complex types that might cause issues */
    case CLOSXP:    /* functions */
    case ENVSXP:    /* environments */
    case PROMSXP:   /* promises */
    case LANGSXP:   /* language objects */
    case SYMSXP:    /* symbols */
    case BUILTINSXP: /* built-in functions */
    case SPECIALSXP: /* special functions */
    case DOTSXP:    /* dot-dot-dot */
    case ANYSXP:    /* any type */
    case EXPRSXP:   /* expression vectors */
    case EXTPTRSXP: /* external pointers */
    case WEAKREFSXP: /* weak references */
    case RAWSXP:    /* raw bytes */
    case S4SXP:     /* S4 objects */
        return json_string("complex-r-object");
        
    default: 
        break;
    }
    
    return json_string("unsupported-type");
}

/* ---------- middleware_init – called once at server start ------------- */
/* Check if R was already initialized using a process-level flag file */
static int check_r_initialized(void) {
    char flag_path[256];
    snprintf(flag_path, sizeof(flag_path), "/tmp/wp_r_initialized_%d", getpid());
    FILE *f = fopen(flag_path, "r");
    if (f) {
        fclose(f);
        return 1;  /* Flag file exists, R was initialized */
    }
    return 0;  /* Flag file doesn't exist */
}

static void mark_r_initialized(void) {
    char flag_path[256];
    snprintf(flag_path, sizeof(flag_path), "/tmp/wp_r_initialized_%d", getpid());
    FILE *f = fopen(flag_path, "w");
    if (f) {
        fprintf(f, "1\n");
        fclose(f);
    }
}

int middleware_init(json_t *config)
{
    (void)config;

    if (r_running) {
        return 0;               /* already marked as running */
    }

    if (!getenv("R_HOME")) {
#ifdef DEFAULT_R_HOME
        setenv("R_HOME", DEFAULT_R_HOME, 1);
#endif
    }

    /* Only try to initialize R if it was never initialized in this process */
    if (!check_r_initialized()) {
        /* Save original signal handlers before R overwrites them */
        sigaction(SIGINT, NULL, &original_sigint_handler);
        sigaction(SIGTERM, NULL, &original_sigterm_handler);
        
        char *argv[] = { "R", "--silent", "--no-save" };
        fprintf(stderr, "R middleware: Initializing R for first time...\n");
        int r_init_result = Rf_initEmbeddedR(3, argv);
        
        if (r_init_result < 0) {
            fprintf(stderr, "R: init failed\n");
            return 1;
        }
        
        /* Restore original signal handlers after R initialization */
        sigaction(SIGINT, &original_sigint_handler, NULL);
        sigaction(SIGTERM, &original_sigterm_handler, NULL);
        fprintf(stderr, "R middleware: Signal handlers restored\n");

        /* Load required packages */
        ParseStatus st;
        
        /* Load jsonlite for JSON handling */
        SEXP jsonlite_code = PROTECT(Rf_mkString("suppressPackageStartupMessages(library(jsonlite))"));
        SEXP jsonlite_expr = PROTECT(R_ParseVector(jsonlite_code, -1, &st, R_NilValue));
        if (st == PARSE_OK) {
            R_tryEval(VECTOR_ELT(jsonlite_expr,0), R_GlobalEnv, NULL);
        }
        UNPROTECT(2);
        
        /* Load ggplot2 for plotting */
        SEXP ggplot_code = PROTECT(Rf_mkString("suppressPackageStartupMessages(library(ggplot2))"));
        SEXP ggplot_expr = PROTECT(R_ParseVector(ggplot_code, -1, &st, R_NilValue));
        if (st == PARSE_OK) {
            R_tryEval(VECTOR_ELT(ggplot_expr,0), R_GlobalEnv, NULL);
        }
        UNPROTECT(2);
        
        mark_r_initialized();
    } else {
        fprintf(stderr, "R middleware: Reusing existing R session...\n");
    }

    r_running = 1;
    return 0;
}

/* ---------- middleware_execute – evaluate user expression ------------- */
json_t *middleware_execute(json_t *input,
                           void   *arena,
                           arena_alloc_func alloc,
                           arena_free_func  free_,
                           const char *filter,
                           json_t *config,
                           char **contentType,
                           json_t *variables)
{
    (void)arena; (void)alloc; (void)free_;
    (void)config; (void)variables;
    if (contentType) *contentType = "application/json";

    if (!r_running)
        return json_pack("{s:s}", "error","R not initialised");

    if (!filter || !*filter)
        filter = ".";                     /* identity – echo .input */

    pthread_mutex_lock(&r_lock);

    /* expose .input to R ------------------------ */
    SEXP r_input = PROTECT(json_to_sexp(input));
    Rf_defineVar(Rf_install(".input"), r_input, R_GlobalEnv);
    
    /* get parsed expression --------------------- */
    SEXP expr = cached_parse(filter);
    if (expr == R_NilValue) {
        UNPROTECT(1);
        pthread_mutex_unlock(&r_lock);
        return json_pack("{s:s}", "error","R parse error");
    }
    
    /* evaluate ---------------------------------- */
    int err = 0;
    SEXP res = R_NilValue;
    
    /* Evaluate all expressions in the vector, keep result of last one */
    R_xlen_t n_exprs = LENGTH(expr);
    for (R_xlen_t i = 0; i < n_exprs; i++) {
        SEXP single_expr = VECTOR_ELT(expr, i);
        res = R_tryEval(single_expr, R_GlobalEnv, &err);
        if (err) {
            break;
        }
    }
    
    json_t *out = NULL;

    if (!err && res != R_NilValue) {
        
        /* common pattern: user returns jsonlite::toJSON(x, auto_unbox=TRUE) */
        if (TYPEOF(res)==STRSXP && XLENGTH(res)==1) {
            const char *txt = CHAR(STRING_ELT(res,0));
            json_error_t jerr;
            out = json_loads(txt, 0, &jerr);
            if (!out) {
                out = json_pack("{s:s,s:s}",
                                "error","json parse failed",
                                "detail", jerr.text);
            }
        } else {
            out = sexp_to_json(res);
        }
    } else {
        out = json_pack("{s:s}","error","R evaluation error");
    }

    UNPROTECT(1);                         /* r_input */
    pthread_mutex_unlock(&r_lock);
    return out;
}

/* ---------- module unload ------------------------------------------------ */
void r_middleware_fini(void)
{
    if (r_running) {
        fprintf(stderr, "R middleware: Preparing for reload (preserving R session)...\n");
        r_running = 0;  /* Mark as not running so it can be reinitialized on next load */
        
        /* Don't clean up parse cache or terminate R - preserve state for performance */
        /* R session and cached expressions will be reused across reloads */
    }
}
