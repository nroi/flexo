FROM nginx:1.17.10

COPY random_small /usr/share/nginx/html/random_small

COPY default.conf /etc/nginx/conf.d/default.conf

COPY create_test_files /root
RUN /root/create_test_files
