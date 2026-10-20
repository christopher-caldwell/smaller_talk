UPDATE turns SET status = 'interrupted', error = 'The app closed before the response completed.' WHERE status = 'running';
