-- Test database schema and data for wp runtime tests

-- Create test tables
CREATE TABLE IF NOT EXISTS teams (
    id SERIAL PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS users (
    id SERIAL PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    email VARCHAR(255) UNIQUE NOT NULL,
    team_id INTEGER REFERENCES teams(id),
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS posts (
    id SERIAL PRIMARY KEY,
    title VARCHAR(255) NOT NULL,
    content TEXT,
    user_id INTEGER REFERENCES users(id),
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Insert test data
INSERT INTO teams (name) VALUES 
    ('Engineering'),
    ('Marketing'),
    ('Sales'),
    ('Support')
ON CONFLICT DO NOTHING;

INSERT INTO users (name, email, team_id) VALUES 
    ('Alice Johnson', 'alice@example.com', 1),
    ('Bob Smith', 'bob@example.com', 1),
    ('Carol Davis', 'carol@example.com', 2),
    ('David Wilson', 'david@example.com', 3),
    ('Eve Brown', 'eve@example.com', 4)
ON CONFLICT DO NOTHING;

INSERT INTO posts (title, content, user_id) VALUES 
    ('First Post', 'This is the first test post', 1),
    ('Second Post', 'This is the second test post', 2),
    ('Third Post', 'This is the third test post', 1),
    ('Fourth Post', 'This is the fourth test post', 3),
    ('Fifth Post', 'This is the fifth test post', 4)
ON CONFLICT DO NOTHING;