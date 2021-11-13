FROM nginx:1.19.1

COPY default.conf /etc/nginx/conf.d/default.conf

COPY create_test_files /root
RUN /root/create_test_files
